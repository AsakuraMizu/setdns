use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    process,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Error, Result, anyhow};
use setdns::{Config, SetDns};
use tokio::time::sleep;

use crate::{
    system_dns,
    test_dns::{Counters, DEFAULT_TEST_DNS_LISTEN_IP, TestDnsServer},
    test_tun::{TUN_DNS_IP, TestTun},
    verifier,
};

pub const OWNER: &str = "test-runner";
pub const PUBLIC_SPLIT_DOMAIN: &str = "example.com";
pub const LOCAL_SPLIT_DOMAIN: &str = "setdns.test";
pub const NON_SPLIT_DOMAIN: &str = "example.org";
pub const PUBLIC_FORWARD_NAME: &str = "www.example.com";
pub const NON_SPLIT_NAME: &str = "www.rust-lang.org";
pub const OVERLAY_IPV4: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 10);
pub const OVERLAY_IPV6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x7365, 0x7464, 0x6e73, 0, 0, 0, 0x10);

const DNS_POLL_ATTEMPTS: usize = 20;
const DNS_POLL_DELAY: Duration = Duration::from_millis(50);

const DYNAMIC_CASE_SPECS: &[DynamicCaseSpec] = &[
    DynamicCaseSpec {
        prefix: "setdns",
        domain: PUBLIC_SPLIT_DOMAIN,
        split_domain: true,
        expectation: NameExpectation::Overlay,
    },
    DynamicCaseSpec {
        prefix: "host",
        domain: LOCAL_SPLIT_DOMAIN,
        split_domain: true,
        expectation: NameExpectation::Overlay,
    },
    DynamicCaseSpec {
        prefix: "forward",
        domain: PUBLIC_SPLIT_DOMAIN,
        split_domain: true,
        expectation: NameExpectation::Forwarded(TestDnsRoute::AllModes),
    },
    DynamicCaseSpec {
        prefix: "outside",
        domain: NON_SPLIT_DOMAIN,
        split_domain: false,
        expectation: NameExpectation::Forwarded(TestDnsRoute::GlobalOnly),
    },
];

const STATIC_CASE_SPECS: &[StaticCaseSpec] = &[
    StaticCaseSpec {
        name: PUBLIC_FORWARD_NAME,
        expectation: NameExpectation::Resolve(ResolutionExpectation::Resolves),
    },
    StaticCaseSpec {
        name: NON_SPLIT_NAME,
        expectation: NameExpectation::Resolve(ResolutionExpectation::Resolves),
    },
];

#[derive(Clone, Copy)]
struct DynamicCaseSpec {
    prefix: &'static str,
    domain: &'static str,
    split_domain: bool,
    expectation: NameExpectation,
}

#[derive(Clone, Copy)]
struct StaticCaseSpec {
    name: &'static str,
    expectation: NameExpectation,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TestDnsRoute {
    AllModes,
    GlobalOnly,
}

#[derive(Clone, Copy)]
enum ResolutionExpectation {
    Resolves,
}

#[derive(Clone, Copy)]
enum NameExpectation {
    Overlay,
    Forwarded(TestDnsRoute),
    Resolve(ResolutionExpectation),
}

struct TestPlan {
    cases: Vec<NameCase>,
    overlay_names: HashSet<String>,
    split_domains: Vec<String>,
}

struct NameCase {
    name: String,
    expectation: NameExpectation,
}

impl TestPlan {
    fn new() -> Self {
        let suffix = unique_run_suffix();
        let mut cases = Vec::with_capacity(DYNAMIC_CASE_SPECS.len() + STATIC_CASE_SPECS.len());
        let mut overlay_names = HashSet::new();
        let mut split_domains = Vec::new();

        for spec in DYNAMIC_CASE_SPECS {
            let name = format!("{}-{suffix}.{}", spec.prefix, spec.domain);
            if matches!(spec.expectation, NameExpectation::Overlay) {
                overlay_names.insert(name.clone());
            }
            if spec.split_domain {
                push_unique_domain(&mut split_domains, spec.domain);
            }
            cases.push(NameCase {
                name,
                expectation: spec.expectation,
            });
        }

        for spec in STATIC_CASE_SPECS {
            cases.push(NameCase {
                name: spec.name.to_owned(),
                expectation: spec.expectation,
            });
        }

        Self {
            cases,
            overlay_names,
            split_domains,
        }
    }
}

fn push_unique_domain(domains: &mut Vec<String>, domain: &str) {
    if !domains.iter().any(|existing| existing == domain) {
        domains.push(domain.to_owned());
    }
}

fn unique_run_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{nanos:x}", process::id())
}

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
    let plan = TestPlan::new();
    let test_dns = TestDnsServer::start(test_dns_ip, parent_dns, plan.overlay_names.clone())
        .await
        .map_err(ScenarioError::unsupported)?;
    let device = tun.as_ref().map(|tun| tun.name().to_owned());
    let config = scenario_config(options.mode, test_dns.listen_ip(), device, &plan);

    let handle =
        SetDns::apply(config).map_err(|error| ScenarioError::unsupported(anyhow!(error)))?;
    let assertion = wait_for_system_dns(|| run_assertions(options.mode, &test_dns, &plan)).await;
    let restore = handle.close();

    if let Err(error) = restore {
        return Err(ScenarioError::restore(anyhow!(error)));
    }

    drop(tun);
    let restore_assertion = wait_for_system_dns(|| run_restore_assertions(&plan)).await;
    if let Err(error) = merge_assertions(assertion, restore_assertion) {
        return Err(ScenarioError::assertion(error));
    }

    Ok(())
}

fn merge_assertions(assertion: Result<()>, restore_assertion: Result<()>) -> Result<()> {
    match (assertion, restore_assertion) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(assertion), Err(restore)) => Err(anyhow!(
            "DNS assertion failed: {assertion:#}; restore assertion also failed: {restore:#}"
        )),
    }
}

async fn wait_for_system_dns<F>(mut assertion: F) -> Result<()>
where
    F: FnMut() -> Result<()>,
{
    let mut last_error = None;
    for attempt in 0..DNS_POLL_ATTEMPTS {
        match assertion() {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 != DNS_POLL_ATTEMPTS {
            sleep(DNS_POLL_DELAY).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("DNS assertion did not run")))
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

fn scenario_config(
    mode: Mode,
    test_dns_ip: IpAddr,
    device: Option<String>,
    plan: &TestPlan,
) -> Config {
    let domains = match mode {
        Mode::Global => Vec::new(),
        Mode::Split => plan.split_domains.clone(),
    };

    Config {
        owner: OWNER.to_owned(),
        servers: vec![test_dns_ip],
        domains,
        device,
    }
}

fn run_assertions(mode: Mode, test_dns: &TestDnsServer, plan: &TestPlan) -> Result<()> {
    for case in &plan.cases {
        assert_case(mode, test_dns, case)?;
    }
    Ok(())
}

fn assert_case(mode: Mode, test_dns: &TestDnsServer, case: &NameCase) -> Result<()> {
    match case.expectation {
        NameExpectation::Overlay => {
            verifier::expect_overlay(&case.name)?;
            let counters = test_dns.counters();
            expect_overlay_counter(&counters, &case.name)
        },
        NameExpectation::Forwarded(route) => {
            let _ = verifier::resolve_system_ips(&case.name);
            let counters = test_dns.counters();
            if route_uses_test_dns(route, mode) {
                expect_forward_counter(&counters, &case.name)
            } else {
                expect_no_name_counter(&counters, &case.name)
            }
        },
        NameExpectation::Resolve(resolution) => assert_resolution(&case.name, resolution),
    }
}

fn assert_resolution(name: &str, resolution: ResolutionExpectation) -> Result<()> {
    match resolution {
        ResolutionExpectation::Resolves => verifier::expect_resolves(name).map(|_| ()),
    }
}

fn route_uses_test_dns(route: TestDnsRoute, mode: Mode) -> bool {
    route == TestDnsRoute::AllModes
        || matches!((route, mode), (TestDnsRoute::GlobalOnly, Mode::Global))
}

fn run_restore_assertions(plan: &TestPlan) -> Result<()> {
    for case in &plan.cases {
        if matches!(case.expectation, NameExpectation::Overlay) {
            verifier::expect_not_overlay(&case.name)?;
        }
    }
    Ok(())
}

fn expect_no_name_counter(counters: &Counters, name: &str) -> Result<()> {
    if counters.by_name.get(name).copied().unwrap_or_default() != 0 {
        anyhow::bail!(
            "test DNS server unexpectedly received a query for {name}; counters: {counters:?}"
        );
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
