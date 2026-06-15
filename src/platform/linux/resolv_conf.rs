use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::IpAddr,
    path::Path,
};

use crate::{Error, Result, config::NormalizedConfig};

const RESOLV_CONF: &str = "/etc/resolv.conf";
const BACKUP: &str = "/etc/resolv.conf.setdns.bak";
const TEMP: &str = "/etc/resolv.conf.setdns.tmp";
const ETC: &str = "/etc";
const HEADER_PREFIX: &str = "# setdns owner: ";

pub(crate) struct SetDns {
    owner: String,
}

#[derive(Debug, thiserror::Error)]
enum ResolvConfError {
    #[error("split DNS is not supported by the /etc/resolv.conf backend")]
    UnsupportedSplitDns,
    #[error("found stale setdns backup without matching owned /etc/resolv.conf")]
    StaleBackup,
    #[error("setdns backup is missing for owner {owner}")]
    MissingBackup { owner: String },
    #[error("/etc/resolv.conf was changed outside setdns before restore for owner {owner}")]
    Trampled { owner: String },
    #[error("permission denied while trying to {operation} {path}")]
    PermissionDenied {
        operation: &'static str,
        path: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("failed to {operation} {path}")]
    Io {
        operation: &'static str,
        path: &'static str,
        #[source]
        source: io::Error,
    },
}

impl From<ResolvConfError> for Error {
    fn from(error: ResolvConfError) -> Self {
        Self::Backend(Box::new(error))
    }
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        if !config.domains.is_empty() {
            return Err(ResolvConfError::UnsupportedSplitDns.into());
        }

        log::debug!(
            "applying Linux /etc/resolv.conf DNS: owner={}, servers={}",
            config.owner,
            config.servers.len()
        );
        cleanup_residual(&config.owner)?;
        if Path::new(BACKUP).exists() {
            return Err(ResolvConfError::StaleBackup.into());
        }

        copy_with_fsync(RESOLV_CONF, BACKUP)?;
        log::debug!("backed up {RESOLV_CONF} to {BACKUP}");
        write_temp(&render_resolv_conf(&config.owner, &config.servers))?;
        rename(TEMP, RESOLV_CONF)?;
        sync_dir(ETC)?;
        log::debug!("replaced {RESOLV_CONF} with setdns-managed content");

        Ok(Self {
            owner: config.owner,
        })
    }

    pub(crate) fn close(self) -> Result<()> {
        let current = read_to_string(RESOLV_CONF)?;
        if !current.starts_with(&owner_header(&self.owner)) {
            return Err(ResolvConfError::Trampled { owner: self.owner }.into());
        }
        if !Path::new(BACKUP).exists() {
            return Err(ResolvConfError::MissingBackup { owner: self.owner }.into());
        }

        log::debug!("restoring {RESOLV_CONF} from {BACKUP}");
        rename(BACKUP, RESOLV_CONF)?;
        sync_dir(ETC)
    }
}

pub(crate) fn is_managed_by_resolved() -> bool {
    let Ok(metadata) = fs::symlink_metadata(RESOLV_CONF) else {
        return false;
    };
    if !metadata.file_type().is_symlink() {
        return false;
    }

    let Ok(target) = fs::read_link(RESOLV_CONF) else {
        return false;
    };
    target.to_string_lossy().contains("systemd/resolve")
}

fn cleanup_residual(owner: &str) -> Result<()> {
    let current = match fs::read_to_string(RESOLV_CONF) {
        Ok(current) => current,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(map_io("read", RESOLV_CONF, source)),
    };
    if !current.starts_with(&owner_header(owner)) {
        return Ok(());
    }
    if !Path::new(BACKUP).exists() {
        return Err(ResolvConfError::MissingBackup {
            owner: owner.to_owned(),
        }
        .into());
    }

    log::debug!("cleaning up residual setdns {RESOLV_CONF} state for owner {owner}");
    rename(BACKUP, RESOLV_CONF)?;
    sync_dir(ETC)
}

fn render_resolv_conf(owner: &str, servers: &[IpAddr]) -> String {
    let mut content = String::new();
    content.push_str(&owner_header(owner));
    content.push_str(
        "# This file was written by setdns and will be restored when the handle closes.\n",
    );
    for server in servers {
        content.push_str("nameserver ");
        content.push_str(&server.to_string());
        content.push('\n');
    }
    content
}

fn owner_header(owner: &str) -> String {
    let mut header = String::with_capacity(HEADER_PREFIX.len() + owner.len() + 1);
    header.push_str(HEADER_PREFIX);
    header.push_str(owner);
    header.push('\n');
    header
}

fn write_temp(content: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(TEMP)
        .map_err(|source| map_io("create", TEMP, source))?;
    file.write_all(content.as_bytes())
        .map_err(|source| map_io("write", TEMP, source))?;
    file.sync_all()
        .map_err(|source| map_io("fsync", TEMP, source))
}

fn copy_with_fsync(from: &'static str, to: &'static str) -> Result<()> {
    fs::copy(from, to).map_err(|source| map_io("copy", to, source))?;
    File::open(to)
        .map_err(|source| map_io("open", to, source))?
        .sync_all()
        .map_err(|source| map_io("fsync", to, source))
}

fn rename(from: &'static str, to: &'static str) -> Result<()> {
    fs::rename(from, to).map_err(|source| map_io("rename", to, source))
}

fn sync_dir(path: &'static str) -> Result<()> {
    File::open(path)
        .map_err(|source| map_io("open", path, source))?
        .sync_all()
        .map_err(|source| map_io("fsync", path, source))
}

fn read_to_string(path: &'static str) -> Result<String> {
    fs::read_to_string(path).map_err(|source| map_io("read", path, source))
}

fn map_io(operation: &'static str, path: &'static str, source: io::Error) -> Error {
    match source.kind() {
        io::ErrorKind::PermissionDenied => ResolvConfError::PermissionDenied {
            operation,
            path,
            source,
        }
        .into(),
        _ => ResolvConfError::Io {
            operation,
            path,
            source,
        }
        .into(),
    }
}
