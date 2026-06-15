use std::{
    fs::{self, File},
    io::{self, Write},
    net::IpAddr,
    path::{Path, PathBuf},
};

use crate::{Error, Result, config::NormalizedConfig};

const RESOLVER_DIR: &str = "/etc/resolver";
const MAXNS: usize = 3;
const TEMP_SUFFIX: &str = ".setdns.tmp";

pub(crate) struct SetDns {
    owner: String,
    domains: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
enum ResolverError {
    #[error("macOS split DNS supports at most {max} nameservers, got {count}")]
    TooManyNameservers { count: usize, max: usize },
    #[error("permission denied while trying to {operation} {path}")]
    PermissionDenied {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("resolver file {path} is not owned by setdns owner {owner}")]
    NotOwned { path: PathBuf, owner: String },
}

impl From<ResolverError> for Error {
    fn from(error: ResolverError) -> Self {
        Self::Backend(Box::new(error))
    }
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        if config.servers.len() > MAXNS {
            return Err(ResolverError::TooManyNameservers {
                count: config.servers.len(),
                max: MAXNS,
            }
            .into());
        }
        log::debug!(
            "applying macOS split DNS resolver files: owner={}, domains={}, servers={}",
            config.owner,
            config.domains.len(),
            config.servers.len()
        );

        cleanup_owner(&config.owner)?;
        fs::create_dir_all(RESOLVER_DIR)
            .map_err(|source| map_io("create", resolver_dir(), source))?;

        let mut domains = Vec::with_capacity(config.domains.len());
        for suffix in &config.domains {
            let content = render_resolver(&config.owner, &suffix.domain, &config.servers);
            write_resolver(&config.owner, &suffix.domain, &content)?;
            domains.push(suffix.domain.clone());
            log::debug!("wrote macOS resolver file for domain {}", suffix.domain);
        }

        crate::platform::macos::state::flush_dns_cache();

        Ok(Self {
            owner: config.owner,
            domains,
        })
    }

    pub(crate) fn close(self) -> Result<()> {
        log::debug!(
            "removing macOS split DNS resolver files: owner={}, domains={}",
            self.owner,
            self.domains.len()
        );
        let mut first_error = None;
        for domain in &self.domains {
            if let Err(err) = remove_owned_file(&resolver_path(domain), &self.owner) {
                first_error.get_or_insert(err);
            }
        }
        if let Err(err) = cleanup_owner(&self.owner) {
            first_error.get_or_insert(err);
        }

        crate::platform::macos::state::flush_dns_cache();

        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }
}

fn cleanup_owner(owner: &str) -> Result<()> {
    let dir = resolver_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(map_io("read", dir, source)),
    };

    for entry in entries {
        let entry = entry.map_err(|source| map_io("read", resolver_dir(), source))?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(TEMP_SUFFIX))
        {
            remove_if_owner_marked(&path, owner)?;
            continue;
        }
        remove_if_owner_marked(&path, owner)?;
    }

    Ok(())
}

fn remove_if_owner_marked(path: &Path, owner: &str) -> Result<()> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) if err.kind() == io::ErrorKind::InvalidData => return Ok(()),
        Err(source) => return Err(map_io("read", path.to_path_buf(), source)),
    };

    if has_owner_header(&content, owner) {
        log::debug!("removing residual macOS resolver file {}", path.display());
        fs::remove_file(path).map_err(|source| map_io("remove", path.to_path_buf(), source))?;
    }

    Ok(())
}

fn remove_owned_file(path: &Path, owner: &str) -> Result<()> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(map_io("read", path.to_path_buf(), source)),
    };

    if !has_owner_header(&content, owner) {
        return Err(ResolverError::NotOwned {
            path: path.to_path_buf(),
            owner: owner.to_owned(),
        }
        .into());
    }

    fs::remove_file(path).map_err(|source| map_io("remove", path.to_path_buf(), source))
}

fn write_resolver(owner: &str, domain: &str, content: &str) -> Result<()> {
    let path = resolver_path(domain);
    ensure_absent_or_owned(&path, owner)?;

    let temp_path = resolver_path(&format!("{domain}{TEMP_SUFFIX}"));
    ensure_absent_or_owned(&temp_path, owner)?;

    let mut file =
        File::create(&temp_path).map_err(|source| map_io("create", temp_path.clone(), source))?;
    file.write_all(content.as_bytes())
        .map_err(|source| map_io("write", temp_path.clone(), source))?;
    file.sync_all()
        .map_err(|source| map_io("sync", temp_path.clone(), source))?;
    drop(file);

    fs::rename(&temp_path, &path).map_err(|source| map_io("rename", path, source))
}

fn ensure_absent_or_owned(path: &Path, owner: &str) -> Result<()> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(map_io("read", path.to_path_buf(), source)),
    };

    if has_owner_header(&content, owner) {
        Ok(())
    } else {
        Err(ResolverError::NotOwned {
            path: path.to_path_buf(),
            owner: owner.to_owned(),
        }
        .into())
    }
}

fn render_resolver(owner: &str, domain: &str, servers: &[IpAddr]) -> String {
    let mut content = String::new();
    content.push_str(&owner_header(owner));
    content.push('\n');
    content.push_str("domain ");
    content.push_str(domain);
    content.push('\n');
    for server in servers {
        content.push_str("nameserver ");
        content.push_str(&server.to_string());
        content.push('\n');
    }
    content
}

fn has_owner_header(content: &str, owner: &str) -> bool {
    content.lines().next() == Some(owner_header(owner).as_str())
}

fn owner_header(owner: &str) -> String {
    format!("# Added by setdns ({owner})")
}

fn resolver_dir() -> PathBuf {
    PathBuf::from(RESOLVER_DIR)
}

fn resolver_path(domain: &str) -> PathBuf {
    resolver_dir().join(domain)
}

fn map_io(operation: &'static str, path: PathBuf, source: io::Error) -> Error {
    match source.kind() {
        io::ErrorKind::PermissionDenied => ResolverError::PermissionDenied {
            operation,
            path,
            source,
        }
        .into(),
        _ => ResolverError::Io {
            operation,
            path,
            source,
        }
        .into(),
    }
}
