use std::net::{IpAddr, ToSocketAddrs};

use anyhow::{Context, Result, anyhow, bail};

use crate::scenario::{OVERLAY_IPV4, OVERLAY_IPV6};

pub fn resolve_system_ips(name: &str) -> Result<Vec<IpAddr>> {
    let ips = (name, 0)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {name} through the system resolver"))?
        .map(|addr| addr.ip())
        .collect::<Vec<_>>();

    Ok(ips)
}

pub fn expect_resolves(name: &str) -> Result<Vec<IpAddr>> {
    let ips = resolve_system_ips(name)?;
    if ips.is_empty() {
        bail!("{name} resolved without returning addresses");
    }
    Ok(ips)
}

pub fn expect_overlay(name: &str) -> Result<()> {
    let ips = expect_resolves(name)?;
    if !contains_overlay(&ips) {
        bail!("{name} did not resolve to an overlay address; got {ips:?}");
    }
    Ok(())
}

pub fn expect_not_overlay(name: &str) -> Result<()> {
    match resolve_system_ips(name) {
        Ok(ips) if contains_overlay(&ips) => Err(anyhow!(
            "{name} still resolves to an overlay address after restore: {ips:?}"
        )),
        Ok(_) | Err(_) => Ok(()),
    }
}

fn contains_overlay(ips: &[IpAddr]) -> bool {
    ips.iter().any(|ip| {
        matches!(ip, IpAddr::V4(ip) if *ip == OVERLAY_IPV4)
            || matches!(ip, IpAddr::V6(ip) if *ip == OVERLAY_IPV6)
    })
}
