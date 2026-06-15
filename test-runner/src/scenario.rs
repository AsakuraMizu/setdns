use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Error, Result, anyhow};
use setdns::{Config, SetDns};

use crate::{
    system_dns,
    test_dns::{Counters, DEFAULT_TEST_DNS_LISTEN_IP, TestDnsServer},
    test_tun::{TUN_DNS_IP, TestTun},
    verifier,
};

pub const OWNER: &str = "test-runner";
pub const PUBLIC_SPLIT_DOMAIN: &str = "example.com";
pub const PUBLIC_OVERLAY_NAME: &str = "setdns.example.com";
pub const PUBLIC_FORWARD_NAME: &str = "www.example.com";
pub const LOCAL_SPLIT_DOMAIN: &str = "setdns.test";
pub const LOCAL_ONLY_NAME: &str = "host.setdns.test";
pub const NON_SPLIT_NAME: &str = "www.rust-lang.org";
pub const OVERLAY_IPV4: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 10);
pub const OVERLAY_IPV6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x7365, 0x7464, 0x6e73, 0, 0, 0, 0x10);

#[derive(Clone, Copy)]
pub enum Mode {
    Global,
    Split,
}

pub struct ScenarioOptions {
    pub mode: Mode,
    pub tun: bool,
    pub parent_dns: Option<IpAddr>,
}

#[derive(Clone, Copy)]
pub enum ExitKind {
    Pass,
    AssertionFailed,
    Unsupported,
    RestoreFailed,
}

pub struct ScenarioError {
    pub kind: ExitKind,
    pub source: Error,
}

impl ScenarioError {
    fn assertion(source: Error) -> Self {
        Self {
            kind: ExitKind::AssertionFailed,
            source,
        }
    }

    fn unsupported(source: Error) -> Self {
        Self {
            kind: ExitKind::Unsupported,
            source,
        }
    }

    fn restore(source: Error) -> Self {
        Self {
            kind: ExitKind::RestoreFailed,
            source,
        }
    }
}

pub async fn run(options: ScenarioOptions) -> std::result::Result<(), ScenarioError> {
    let test_dns_ip = if options.tun {
        TUN_DNS_IP
    } else {
        DEFAULT_TEST_DNS_LISTEN_IP
    };
    let parent_dns = system_dns::select_parent_dns(options.parent_dns, test_dns_ip)
        .map_err(ScenarioError::unsupported)?;
    let tun = create_tun_if_requested(options.tun)?;
    let test_dns = TestDnsServer::start(test_dns_ip, parent_dns)
        .await
        .map_err(ScenarioError::unsupported)?;
    let device = tun.as_ref().map(|tun| tun.name().to_owned());
    let config = scenario_config(options.mode, test_dns.listen_ip(), device);

    let handle =
        SetDns::apply(config).map_err(|error| ScenarioError::unsupported(anyhow!(error)))?;
    let assertion = run_assertions(options.mode, &test_dns);
    let restore = handle.close();

    if let Err(error) = restore {
        return Err(ScenarioError::restore(anyhow!(error)));
    }

    drop(tun);
    if let Err(error) = assertion.and_then(|()| run_restore_assertions(options.mode)) {
        return Err(ScenarioError::assertion(error));
    }

    Ok(())
}

fn create_tun_if_requested(enabled: bool) -> std::result::Result<Option<TestTun>, ScenarioError> {
    if enabled {
        TestTun::create()
            .map(Some)
            .map_err(ScenarioError::unsupported)
    } else {
        Ok(None)
    }
}

fn scenario_config(mode: Mode, test_dns_ip: IpAddr, device: Option<String>) -> Config {
    let domains = match mode {
        Mode::Global => Vec::new(),
        Mode::Split => vec![
            PUBLIC_SPLIT_DOMAIN.to_owned(),
            LOCAL_SPLIT_DOMAIN.to_owned(),
        ],
    };

    Config {
        owner: OWNER.to_owned(),
        servers: vec![test_dns_ip],
        domains,
        device,
    }
}

fn run_assertions(mode: Mode, test_dns: &TestDnsServer) -> Result<()> {
    match mode {
        Mode::Global => assert_global(test_dns),
        Mode::Split => assert_split(test_dns),
    }
}

fn assert_global(test_dns: &TestDnsServer) -> Result<()> {
    verifier::expect_overlay(PUBLIC_OVERLAY_NAME)?;
    verifier::expect_resolves(PUBLIC_FORWARD_NAME)?;

    let counters = test_dns.counters();
    expect_overlay_counter(&counters, PUBLIC_OVERLAY_NAME)?;
    expect_forward_counter(&counters, PUBLIC_FORWARD_NAME)?;
    Ok(())
}

fn assert_split(test_dns: &TestDnsServer) -> Result<()> {
    verifier::expect_overlay(PUBLIC_OVERLAY_NAME)?;
    verifier::expect_overlay(LOCAL_ONLY_NAME)?;
    verifier::expect_resolves(PUBLIC_FORWARD_NAME)?;
    verifier::expect_resolves(NON_SPLIT_NAME)?;

    let counters = test_dns.counters();
    expect_name_counter(&counters, PUBLIC_OVERLAY_NAME)?;
    expect_name_counter(&counters, LOCAL_ONLY_NAME)?;
    expect_overlay_counter(&counters, PUBLIC_OVERLAY_NAME)?;
    expect_overlay_counter(&counters, LOCAL_ONLY_NAME)?;
    expect_forward_counter(&counters, PUBLIC_FORWARD_NAME)?;
    Ok(())
}

fn run_restore_assertions(mode: Mode) -> Result<()> {
    verifier::expect_not_overlay(PUBLIC_OVERLAY_NAME)?;
    if matches!(mode, Mode::Split) {
        verifier::expect_not_overlay(LOCAL_ONLY_NAME)?;
    }
    Ok(())
}

fn expect_name_counter(counters: &Counters, name: &str) -> Result<()> {
    if counters.by_name.get(name).copied().unwrap_or_default() == 0 {
        anyhow::bail!("test DNS server did not receive a query for {name}; counters: {counters:?}");
    }
    Ok(())
}

fn expect_overlay_counter(counters: &Counters, name: &str) -> Result<()> {
    expect_name_counter(counters, name)?;
    if counters
        .overlay_by_name
        .get(name)
        .copied()
        .unwrap_or_default()
        == 0
    {
        anyhow::bail!("test DNS server did not answer {name} locally; counters: {counters:?}");
    }
    Ok(())
}

fn expect_forward_counter(counters: &Counters, name: &str) -> Result<()> {
    expect_name_counter(counters, name)?;
    if counters
        .forwarded_by_name
        .get(name)
        .copied()
        .unwrap_or_default()
        == 0
    {
        anyhow::bail!("test DNS server did not forward {name}; counters: {counters:?}");
    }
    Ok(())
}
