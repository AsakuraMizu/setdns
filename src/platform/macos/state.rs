use std::process::Command;

pub(crate) fn flush_dns_cache() {
    let _ = Command::new("dscacheutil").arg("-flushcache").status();
}
