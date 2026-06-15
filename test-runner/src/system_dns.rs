use std::net::{IpAddr, Ipv4Addr};

use anyhow::{Context, Result, bail};
use hickory_resolver::system_conf::read_system_conf;

const SYSTEMD_RESOLVED_STUBS: [Ipv4Addr; 2] =
    [Ipv4Addr::new(127, 0, 0, 53), Ipv4Addr::new(127, 0, 0, 54)];

pub fn select_parent_dns(override_ip: Option<IpAddr>, test_dns_ip: IpAddr) -> Result<Vec<IpAddr>> {
    let parents = match override_ip {
        Some(ip) => vec![ip],
        None => {
            let (config, _opts) =
                read_system_conf().context("failed to read system DNS configuration")?;
            config
                .name_servers()
                .iter()
                .map(|server| server.ip)
                .collect::<Vec<_>>()
        },
    };

    validate_parent_dns(&parents, test_dns_ip)?;
    Ok(parents)
}

fn validate_parent_dns(parents: &[IpAddr], test_dns_ip: IpAddr) -> Result<()> {
    if parents.is_empty() {
        bail!("no parent DNS servers were discovered; pass --parent-dns <ip>");
    }

    for parent in parents {
        if *parent == test_dns_ip {
            bail!("parent DNS {parent} points back to the test DNS server; pass --parent-dns <ip>");
        }

        if matches!(parent, IpAddr::V4(ip) if SYSTEMD_RESOLVED_STUBS.contains(ip)) {
            bail!("parent DNS {parent} is a local systemd-resolved stub; pass --parent-dns <ip>");
        }
    }

    Ok(())
}
