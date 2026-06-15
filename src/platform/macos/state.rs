use std::process::Command;

pub(crate) fn flush_dns_cache() {
    match Command::new("dscacheutil").arg("-flushcache").status() {
        Ok(status) if status.success() => log::debug!("flushed macOS DNS cache"),
        Ok(status) => {
            log::warn!("failed to flush macOS DNS cache: dscacheutil exited with {status}")
        },
        Err(err) => log::warn!("failed to launch dscacheutil to flush macOS DNS cache: {err}"),
    }
}
