use std::process::Command;

pub(crate) fn flush_dns_cache() {
    run_command("dscacheutil", &["-flushcache"], "flush macOS DNS cache");
    run_command(
        "killall",
        &["-HUP", "mDNSResponder"],
        "reload mDNSResponder",
    );
}

fn run_command(command: &str, args: &[&str], action: &'static str) {
    match Command::new(command).args(args).status() {
        Ok(status) if status.success() => log::debug!("{action} succeeded"),
        Ok(status) => log::warn!("failed to {action}: {command} exited with {status}"),
        Err(err) => log::warn!("failed to launch {command} to {action}: {err}"),
    }
}
