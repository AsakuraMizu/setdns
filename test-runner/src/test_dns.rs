use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use hickory_proto::{
    op::{Message, MessageType, ResponseCode},
    rr::{
        Name, RData, Record, RecordType,
        rdata::{A, AAAA},
    },
};
use tokio::{
    net::UdpSocket,
    task::JoinHandle,
    time::{Duration, timeout},
};

use crate::scenario::{LOCAL_ONLY_NAME, OVERLAY_IPV4, OVERLAY_IPV6, PUBLIC_OVERLAY_NAME};

const DNS_PORT: u16 = 53;
pub const DEFAULT_TEST_DNS_LISTEN_IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
const MAX_DNS_PACKET: usize = 512;
const FORWARD_TIMEOUT: Duration = Duration::from_secs(5);
const TEST_RECORD_TTL: u32 = 0;

#[derive(Clone, Debug, Default)]
pub struct Counters {
    pub overlay_answers: u64,
    pub forwarded_queries: u64,
    pub overlay_by_name: HashMap<String, u64>,
    pub forwarded_by_name: HashMap<String, u64>,
    pub by_name: HashMap<String, u64>,
    pub by_type: HashMap<String, u64>,
}

pub struct TestDnsServer {
    listen_ip: IpAddr,
    counters: Arc<Mutex<Counters>>,
    task: JoinHandle<()>,
}

impl TestDnsServer {
    pub async fn start(listen_ip: IpAddr, parent_dns: Vec<IpAddr>) -> Result<Self> {
        let socket = UdpSocket::bind(SocketAddr::new(listen_ip, DNS_PORT))
            .await
            .with_context(|| format!("failed to bind test DNS server to {listen_ip}:{DNS_PORT}"))?;
        let counters = Arc::new(Mutex::new(Counters::default()));
        let task_counters = Arc::clone(&counters);
        let task = tokio::spawn(async move {
            run_dns_loop(socket, parent_dns, task_counters).await;
        });

        Ok(Self {
            listen_ip,
            counters,
            task,
        })
    }

    pub fn listen_ip(&self) -> IpAddr {
        self.listen_ip
    }

    pub fn counters(&self) -> Counters {
        self.counters
            .lock()
            .expect("test DNS counters mutex poisoned")
            .clone()
    }
}

impl Drop for TestDnsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_dns_loop(socket: UdpSocket, parent_dns: Vec<IpAddr>, counters: Arc<Mutex<Counters>>) {
    let mut buf = [0_u8; MAX_DNS_PACKET];
    loop {
        let (len, peer) = match socket.recv_from(&mut buf).await {
            Ok(packet) => packet,
            Err(error) => {
                tracing::warn!("failed to receive DNS query: {error}");
                continue;
            },
        };
        let packet = &buf[..len];
        let response = match handle_query(packet, &parent_dns, &counters).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!("failed to handle DNS query: {error:#}");
                continue;
            },
        };
        if let Err(error) = socket.send_to(&response, peer).await {
            tracing::warn!("failed to send DNS response: {error}");
        }
    }
}

async fn handle_query(
    packet: &[u8],
    parent_dns: &[IpAddr],
    counters: &Arc<Mutex<Counters>>,
) -> Result<Vec<u8>> {
    let request = Message::from_vec(packet).context("failed to decode DNS query")?;
    let Some(query) = request.queries.first() else {
        return error_response(&request, ResponseCode::FormErr);
    };

    let name = normalize_name(query.name());
    let record_type = query.query_type();
    increment_query_counters(counters, &name, record_type);

    if is_overlay_name(&name) {
        let mut response = Message::new(
            request.metadata.id,
            MessageType::Response,
            request.metadata.op_code,
        );
        response.metadata.recursion_desired = request.metadata.recursion_desired;
        response.metadata.recursion_available = true;
        response.metadata.authoritative = true;
        response.add_query(query.clone());
        add_overlay_answers(&mut response, query.name().clone(), record_type);
        increment_overlay_counter(counters, &name);
        return response
            .to_vec()
            .context("failed to encode overlay DNS response");
    }

    increment_forwarded_counter(counters, &name);
    forward_query(packet, parent_dns).await
}

fn error_response(request: &Message, code: ResponseCode) -> Result<Vec<u8>> {
    let response = Message::error_msg(request.metadata.id, request.metadata.op_code, code);
    response
        .to_vec()
        .context("failed to encode DNS error response")
}

fn add_overlay_answers(response: &mut Message, name: Name, record_type: RecordType) {
    if matches!(record_type, RecordType::A | RecordType::ANY) {
        response.add_answer(Record::from_rdata(
            name.clone(),
            TEST_RECORD_TTL,
            RData::A(A(OVERLAY_IPV4)),
        ));
    }
    if matches!(record_type, RecordType::AAAA | RecordType::ANY) {
        response.add_answer(Record::from_rdata(
            name,
            TEST_RECORD_TTL,
            RData::AAAA(AAAA(OVERLAY_IPV6)),
        ));
    }
}

async fn forward_query(packet: &[u8], parent_dns: &[IpAddr]) -> Result<Vec<u8>> {
    let mut response = [0_u8; MAX_DNS_PACKET];

    for parent in parent_dns {
        let bind_addr = if parent.is_ipv6() {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
        };
        let Ok(socket) = UdpSocket::bind(bind_addr).await else {
            continue;
        };
        let parent_addr = SocketAddr::new(*parent, DNS_PORT);
        if socket.send_to(packet, parent_addr).await.is_err() {
            continue;
        }
        if let Ok(Ok((len, _))) = timeout(FORWARD_TIMEOUT, socket.recv_from(&mut response)).await {
            return Ok(response[..len].to_vec());
        }
    }

    anyhow::bail!("all parent DNS servers failed to answer forwarded query")
}

fn increment_query_counters(counters: &Arc<Mutex<Counters>>, name: &str, record_type: RecordType) {
    let mut counters = counters.lock().expect("test DNS counters mutex poisoned");
    *counters.by_name.entry(name.to_owned()).or_default() += 1;
    *counters.by_type.entry(record_type.to_string()).or_default() += 1;
}

fn increment_overlay_counter(counters: &Arc<Mutex<Counters>>, name: &str) {
    let mut counters = counters.lock().expect("test DNS counters mutex poisoned");
    counters.overlay_answers += 1;
    *counters.overlay_by_name.entry(name.to_owned()).or_default() += 1;
}

fn increment_forwarded_counter(counters: &Arc<Mutex<Counters>>, name: &str) {
    let mut counters = counters.lock().expect("test DNS counters mutex poisoned");
    counters.forwarded_queries += 1;
    *counters
        .forwarded_by_name
        .entry(name.to_owned())
        .or_default() += 1;
}

fn normalize_name(name: &Name) -> String {
    name.to_ascii().trim_end_matches('.').to_ascii_lowercase()
}

fn is_overlay_name(name: &str) -> bool {
    name == PUBLIC_OVERLAY_NAME || name == LOCAL_ONLY_NAME
}
