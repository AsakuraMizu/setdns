use std::net::IpAddr;

use addr::parse_dns_name;

use crate::{Error, Result};

/// DNS configuration applied by [`crate::SetDns`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Owner identifier used to mark system DNS state written by this crate.
    ///
    /// Must be 1 to 64 ASCII bytes and contain only letters, digits, `.`, `_`,
    /// and `-`.
    pub owner: String,
    /// DNS servers to use while the handle is alive.
    pub servers: Vec<IpAddr>,
    /// DNS suffixes for split DNS. Empty means global DNS.
    ///
    /// Suffixes are ASCII DNS names such as `corp.internal`. A leading `*.` is
    /// accepted to express wildcard intent, so `*.corp.internal` normalizes to
    /// the same suffix as `corp.internal`.
    pub domains: Vec<String>,
    /// Optional platform target, such as an interface name.
    ///
    /// Linux treats this as an interface name and requires it for split DNS.
    /// macOS treats it as a BSD interface name for global DNS and ignores it
    /// for split DNS. Windows intentionally ignores this field and applies
    /// global NRPT rules.
    pub device: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NormalizedConfig {
    pub(crate) owner: String,
    pub(crate) servers: Vec<IpAddr>,
    pub(crate) domains: Vec<DomainSuffix>,
    pub(crate) device: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DomainSuffix {
    pub(crate) domain: String,
    pub(crate) wildcard: bool,
}

impl Config {
    pub(crate) fn normalize(self) -> Result<NormalizedConfig> {
        validate_owner(&self.owner)?;

        if self.servers.is_empty() {
            return Err(Error::InvalidConfig("servers must not be empty".to_owned()));
        }

        let domains = normalize_domains(self.domains)?;

        Ok(NormalizedConfig {
            owner: self.owner,
            servers: self.servers,
            domains,
            device: self.device,
        })
    }
}

fn validate_owner(owner: &str) -> Result<()> {
    if owner.is_empty() {
        return Err(Error::InvalidConfig("owner must not be empty".to_owned()));
    }

    if owner.len() > 64 {
        return Err(Error::InvalidConfig(
            "owner must be at most 64 bytes".to_owned(),
        ));
    }

    if !owner.is_ascii() {
        return Err(Error::InvalidConfig("owner must be ASCII".to_owned()));
    }

    if !owner
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(Error::InvalidConfig(
            "owner may only contain ASCII letters, digits, '.', '_', and '-'".to_owned(),
        ));
    }

    Ok(())
}

fn normalize_domains(domains: Vec<String>) -> Result<Vec<DomainSuffix>> {
    let mut suffixes = Vec::with_capacity(domains.len());
    for domain in domains {
        suffixes.push(parse_domain_suffix(&domain)?);
    }

    suffixes.sort_unstable_by(|left, right| {
        label_count(&left.domain)
            .cmp(&label_count(&right.domain))
            .then_with(|| left.domain.cmp(&right.domain))
            .then_with(|| left.wildcard.cmp(&right.wildcard))
    });

    let mut coalesced: Vec<DomainSuffix> = Vec::with_capacity(suffixes.len());
    for suffix in suffixes {
        if coalesced.iter().any(|kept| {
            suffix.domain == kept.domain || is_child_suffix(&suffix.domain, &kept.domain)
        }) {
            continue;
        }
        coalesced.push(suffix);
    }

    Ok(coalesced)
}

fn parse_domain_suffix(input: &str) -> Result<DomainSuffix> {
    let (domain, wildcard) = if let Some(domain) = input.strip_prefix("*.") {
        (domain, true)
    } else {
        (input, false)
    };

    validate_domain(domain)?;

    Ok(DomainSuffix {
        domain: domain.to_ascii_lowercase(),
        wildcard,
    })
}

fn validate_domain(domain: &str) -> Result<()> {
    if domain.is_empty()
        || domain == "."
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return Err(Error::InvalidConfig(
            "domain must contain non-empty labels separated by '.'".to_owned(),
        ));
    }

    if !domain.is_ascii() {
        return Err(Error::InvalidConfig("domain must be ASCII".to_owned()));
    }

    if domain.contains('*') {
        return Err(Error::InvalidConfig(
            "wildcard domains must use the '*.' prefix".to_owned(),
        ));
    }

    parse_dns_name(domain).map_err(|source| {
        Error::InvalidConfig(format!("invalid domain suffix '{domain}': {source}"))
    })?;

    Ok(())
}

fn label_count(domain: &str) -> usize {
    domain.bytes().filter(|byte| *byte == b'.').count() + 1
}

fn is_child_suffix(candidate: &str, parent: &str) -> bool {
    candidate.len() > parent.len()
        && candidate.ends_with(parent)
        && candidate.as_bytes()[candidate.len() - parent.len() - 1] == b'.'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suffixes(input: &[&str]) -> Result<Vec<DomainSuffix>> {
        normalize_domains(input.iter().map(|domain| (*domain).to_owned()).collect())
    }

    #[test]
    fn owner_accepts_documented_charset() {
        assert!(validate_owner("setdns.Owner_1-2").is_ok());
        assert!(validate_owner("").is_err());
        assert!(validate_owner("owner/name").is_err());
        assert!(validate_owner("é").is_err());
        assert!(validate_owner("a".repeat(65).as_str()).is_err());
    }

    #[test]
    fn domain_normalizes_plain_and_wildcard_suffixes() {
        assert_eq!(
            parse_domain_suffix("*.Corp.Internal").unwrap(),
            DomainSuffix {
                domain: "corp.internal".to_owned(),
                wildcard: true,
            }
        );
        assert_eq!(
            parse_domain_suffix("Corp.Internal").unwrap(),
            DomainSuffix {
                domain: "corp.internal".to_owned(),
                wildcard: false,
            }
        );
    }

    #[test]
    fn domain_accepts_dns_service_labels() {
        assert_eq!(
            parse_domain_suffix("_tcp.Corp.Internal").unwrap(),
            DomainSuffix {
                domain: "_tcp.corp.internal".to_owned(),
                wildcard: false,
            }
        );
    }

    #[test]
    fn domain_rejects_invalid_inputs() {
        for domain in [
            "",
            ".",
            "*",
            ".corp.internal",
            "corp.internal.",
            "*.",
            "corp..internal",
            "café.example",
        ] {
            assert!(parse_domain_suffix(domain).is_err(), "{domain}");
        }
    }

    #[test]
    fn domain_coalescing_dedupes_and_removes_children() {
        assert_eq!(
            suffixes(&[
                "*.corp.internal",
                "dev.corp.internal",
                "corp.internal",
                "svc.company.net",
                "a.dev.corp.internal",
            ])
            .unwrap(),
            vec![
                DomainSuffix {
                    domain: "corp.internal".to_owned(),
                    wildcard: false,
                },
                DomainSuffix {
                    domain: "svc.company.net".to_owned(),
                    wildcard: false,
                },
            ]
        );
    }
}
