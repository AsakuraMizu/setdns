use std::net::IpAddr;

use windows::Win32::System::Com::CoCreateGuid;
use windows_registry::LOCAL_MACHINE;

use super::braced_guid;
use crate::{Error, Result};

const NAMESPACE_CHUNK_SIZE: usize = 50;
const NRPT_BASE: &str = r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig";
const STATE_BASE: &str = r"SOFTWARE\setdns\Windows\Nrpt";
const OWNER_VALUE: &str = "setdns_owner";
const CONFIG_OPTIONS_USE_GENERIC_DNS_SERVERS: u32 = 0x8;

#[derive(Debug, thiserror::Error)]
enum NrptError {
    #[error("failed to create NRPT rule id: {0}")]
    CreateRuleId(String),
    #[error("failed to write Windows NRPT registry state: {0}")]
    Registry(String),
}

impl From<NrptError> for Error {
    fn from(error: NrptError) -> Self {
        Self::Backend(Box::new(error))
    }
}

pub(crate) struct RuleSet {
    owner: String,
    rule_ids: Vec<String>,
}

impl RuleSet {
    pub(crate) fn apply(owner: &str, servers: &[IpAddr], namespaces: &[String]) -> Result<Self> {
        cleanup_owner(owner);

        let server_list = server_list(servers);
        let mut rule_ids = Vec::new();

        for chunk in namespaces.chunks(NAMESPACE_CHUNK_SIZE) {
            let rule_id = create_rule_id()?;
            if let Err(error) = write_rule(owner, &rule_id, &server_list, chunk) {
                remove_rules(&rule_ids);
                return Err(error);
            }
            rule_ids.push(rule_id);
        }

        if let Err(error) = write_state(owner, &rule_ids) {
            remove_rules(&rule_ids);
            return Err(error);
        }

        Ok(Self {
            owner: owner.to_owned(),
            rule_ids,
        })
    }

    pub(crate) fn close(self) -> Result<()> {
        remove_rules_checked(&self.rule_ids)?;
        remove_state_checked(&self.owner)
    }

    pub(crate) fn len(&self) -> usize {
        self.rule_ids.len()
    }
}

pub(crate) fn namespaces_for_global_or_split(
    domains: &[crate::config::DomainSuffix],
) -> Vec<String> {
    if domains.is_empty() {
        return vec![".".to_owned()];
    }

    domains
        .iter()
        .map(|domain| format!(".{}", domain.domain))
        .collect()
}

fn create_rule_id() -> Result<String> {
    let guid =
        unsafe { CoCreateGuid() }.map_err(|error| NrptError::CreateRuleId(error.to_string()))?;
    Ok(braced_guid(&guid))
}

fn write_rule(owner: &str, rule_id: &str, servers: &str, domains: &[String]) -> Result<()> {
    let path = format!(r"{NRPT_BASE}\{rule_id}");
    let key = LOCAL_MACHINE
        .create(path)
        .map_err(|error| NrptError::Registry(error.to_string()))?;
    let domain_refs: Vec<&str> = domains.iter().map(String::as_str).collect();

    key.set_u32("Version", 1)
        .map_err(|error| NrptError::Registry(error.to_string()))?;
    key.set_multi_string("Name", &domain_refs)
        .map_err(|error| NrptError::Registry(error.to_string()))?;
    key.set_string("GenericDNSServers", servers)
        .map_err(|error| NrptError::Registry(error.to_string()))?;
    key.set_u32("ConfigOptions", CONFIG_OPTIONS_USE_GENERIC_DNS_SERVERS)
        .map_err(|error| NrptError::Registry(error.to_string()))?;
    key.set_string(OWNER_VALUE, owner)
        .map_err(|error| NrptError::Registry(error.to_string()))?;

    Ok(())
}

fn write_state(owner: &str, rule_ids: &[String]) -> Result<()> {
    let key = LOCAL_MACHINE
        .create(format!(r"{STATE_BASE}\{owner}"))
        .map_err(|error| NrptError::Registry(error.to_string()))?;
    let rule_refs: Vec<&str> = rule_ids.iter().map(String::as_str).collect();

    key.set_multi_string("RuleIds", &rule_refs)
        .map_err(|error| NrptError::Registry(error.to_string()))
        .map_err(Into::into)
}

fn cleanup_owner(owner: &str) {
    let Ok(key) = LOCAL_MACHINE.open(format!(r"{STATE_BASE}\{owner}")) else {
        return;
    };
    let Ok(rule_ids) = key.get_multi_string("RuleIds") else {
        log::warn!(
            "failed to read Windows NRPT state for owner {owner}; cleaning owner-tagged rules"
        );
        remove_owner_tagged_rules(owner);
        remove_state(owner);
        return;
    };

    remove_rules(&rule_ids);
    remove_state(owner);
}

fn remove_rules(rule_ids: &[String]) {
    for rule_id in rule_ids {
        let path = format!(r"{NRPT_BASE}\{rule_id}");
        if let Err(error) = LOCAL_MACHINE.remove_tree(path) {
            log::debug!("failed to remove Windows NRPT rule {rule_id}: {error}");
        }
    }
}

fn remove_owner_tagged_rules(owner: &str) {
    let Ok(base) = LOCAL_MACHINE.open(NRPT_BASE) else {
        return;
    };
    let Ok(rule_ids) = base.keys() else {
        return;
    };

    for rule_id in rule_ids {
        let path = format!(r"{NRPT_BASE}\{rule_id}");
        let Ok(rule) = LOCAL_MACHINE.open(&path) else {
            continue;
        };
        if rule.get_string(OWNER_VALUE).ok().as_deref() == Some(owner)
            && let Err(error) = LOCAL_MACHINE.remove_tree(path)
        {
            log::debug!("failed to remove owner-tagged Windows NRPT rule {rule_id}: {error}");
        }
    }
}

fn remove_rules_checked(rule_ids: &[String]) -> Result<()> {
    for rule_id in rule_ids {
        let path = format!(r"{NRPT_BASE}\{rule_id}");
        LOCAL_MACHINE
            .remove_tree(path)
            .map_err(|error| NrptError::Registry(error.to_string()))?;
    }
    Ok(())
}

fn remove_state_checked(owner: &str) -> Result<()> {
    LOCAL_MACHINE
        .remove_tree(format!(r"{STATE_BASE}\{owner}"))
        .map_err(|error| NrptError::Registry(error.to_string()))
        .map_err(Into::into)
}

fn remove_state(owner: &str) {
    if let Err(error) = LOCAL_MACHINE.remove_tree(format!(r"{STATE_BASE}\{owner}")) {
        log::debug!("failed to remove Windows NRPT state for owner {owner}: {error}");
    }
}

fn server_list(servers: &[IpAddr]) -> String {
    let mut result = String::new();
    for server in servers {
        if !result.is_empty() {
            result.push(';');
        }
        result.push_str(&server.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::{namespaces_for_global_or_split, server_list};
    use crate::config::DomainSuffix;

    #[test]
    fn maps_global_namespace_to_dot() {
        assert_eq!(namespaces_for_global_or_split(&[]), vec!["."]);
    }

    #[test]
    fn maps_split_namespaces_to_dot_prefixed_suffixes() {
        let domains = vec![
            DomainSuffix {
                domain: "corp.internal".to_owned(),
                wildcard: false,
            },
            DomainSuffix {
                domain: "example.net".to_owned(),
                wildcard: true,
            },
        ];

        assert_eq!(
            namespaces_for_global_or_split(&domains),
            vec![".corp.internal", ".example.net"]
        );
    }

    #[test]
    fn joins_servers_for_nrpt_registry() {
        assert_eq!(
            server_list(&[
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ]),
            "1.1.1.1;::1"
        );
    }
}
