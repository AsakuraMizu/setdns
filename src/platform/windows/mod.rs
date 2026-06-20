mod interface;
mod nrpt;

use windows::core::GUID;

use crate::{Result, config::NormalizedConfig};

pub(crate) struct SetDns {
    nrpt_rules: nrpt::RuleSet,
    interface_dns: Option<interface::InterfaceDns>,
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        let namespaces = nrpt::namespaces_for_global_or_split(&config.domains);
        log::debug!(
            "applying Windows NRPT DNS rules: owner={}, servers={}, namespaces={}",
            config.owner,
            config.servers.len(),
            namespaces.len()
        );

        let mut interface_dns = None;
        if config.domains.is_empty() {
            if let Some(device) = &config.device {
                log::debug!("applying Windows interface DNS on '{device}'");
                interface_dns = interface::InterfaceDns::apply(device, &config.servers)?;
            }
        } else if let Some(device) = &config.device {
            log::debug!("ignoring Windows device field '{device}' for split DNS");
        }

        let nrpt_rules = match nrpt::RuleSet::apply(&config.owner, &config.servers, &namespaces) {
            Ok(rules) => rules,
            Err(error) => {
                if let Some(interface_dns) = interface_dns
                    && let Err(restore_error) = interface_dns.close()
                {
                    log::warn!(
                        "failed to restore Windows interface DNS after NRPT apply failure: \
                         {restore_error}"
                    );
                }
                return Err(error);
            },
        };
        log::debug!("created {} Windows NRPT rules", nrpt_rules.len());
        flush_dns_cache();

        Ok(Self {
            nrpt_rules,
            interface_dns,
        })
    }

    pub(crate) fn close(self) -> Result<()> {
        let nrpt_result = self.nrpt_rules.close();
        let interface_result = self.interface_dns.map(interface::InterfaceDns::close);

        if let Some(Err(error)) = &interface_result {
            if nrpt_result.is_err() {
                log::warn!("failed to restore Windows interface DNS during close: {error}");
            }
        }

        flush_dns_cache();
        nrpt_result.and(interface_result.unwrap_or(Ok(())))
    }
}

fn flush_dns_cache() {
    match std::process::Command::new("ipconfig")
        .arg("/flushdns")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) if status.success() => log::debug!("flushed Windows DNS client cache"),
        Ok(status) => log::debug!("ipconfig /flushdns exited with status {status}"),
        Err(error) => log::debug!("failed to run ipconfig /flushdns: {error}"),
    }
}

fn braced_guid(guid: &GUID) -> String {
    format!(
        "{{{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

#[cfg(test)]
mod tests {
    use windows::core::GUID;

    use super::braced_guid;

    #[test]
    fn formats_registry_guid_with_braces() {
        let guid = GUID {
            data1: 0x12345678,
            data2: 0x9abc,
            data3: 0xdef0,
            data4: [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0],
        };

        assert_eq!(braced_guid(&guid), "{12345678-9abc-def0-1234-56789abcdef0}");
    }
}
