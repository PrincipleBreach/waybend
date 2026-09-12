//! Minimal authoritative DNS server for deterministic rebinding tests.
//!
//! Names are encoded below the configured zone using one of these forms:
//!
//! - `static.<zone>`, `alt.<zone>`, and `<count>.toctou.<zone>` use the two
//!   configured addresses.
//! - `<ip>.static.<zone>` always returns `ip`.
//! - `<first>.<second>.alt.<zone>` alternates between the addresses.
//! - `<first>.<second>.<count>.toctou.<zone>` returns `first` for `count`
//!   queries, then returns `second`.
//!
//! IPv4 addresses use dashes instead of dots (`127-0-0-1`). IPv6 addresses
//! are 32 lowercase hexadecimal digits with no separators.

use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    num::NonZeroUsize,
    str::FromStr,
    sync::{Arc, Mutex},
};

use hickory_proto::{
    op::{Message, MessageType, OpCode, ResponseCode},
    rr::{
        Name, RData, Record, RecordType,
        rdata::{A, AAAA, NS, SOA},
    },
    serialize::binary::{BinDecodable, BinEncodable, DecodeError},
};
use lru::LruCache;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Semaphore, watch},
    time::{Duration, timeout},
};

#[derive(Debug, Clone)]
pub struct DnsConfig {
    pub udp_bind: SocketAddr,
    pub tcp_bind: SocketAddr,
    pub zone: Name,
    pub ttl: u32,
    pub first_ip: IpAddr,
    pub second_ip: IpAddr,
}

impl DnsConfig {
    pub fn new(bind: SocketAddr, zone: &str, ttl: u32) -> Result<Self, DnsError> {
        Self::with_bindings(
            bind,
            bind,
            zone,
            ttl,
            "192.0.2.1".parse().expect("static IP address"),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
    }

    pub fn with_bindings(
        udp_bind: SocketAddr,
        tcp_bind: SocketAddr,
        zone: &str,
        ttl: u32,
        first_ip: IpAddr,
        second_ip: IpAddr,
    ) -> Result<Self, DnsError> {
        let zone = Name::from_ascii(format!("{}.", zone.trim_end_matches('.')))
            .map_err(|_| DnsError::InvalidZone)?;
        if zone.is_root() {
            return Err(DnsError::InvalidZone);
        }
        Ok(Self {
            udp_bind,
            tcp_bind,
            zone,
            ttl,
            first_ip,
            second_ip,
        })
    }
}

impl TryFrom<&crate::config::DnsConfig> for DnsConfig {
    type Error = DnsError;

    fn try_from(config: &crate::config::DnsConfig) -> Result<Self, Self::Error> {
        let domain = config.domain.as_deref().ok_or(DnsError::InvalidZone)?;
        Self::with_bindings(
            config.udp_listen,
            config.tcp_listen,
            domain,
            config.ttl,
            config.first_ip,
            config.second_ip,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsEvidence {
    pub source: SocketAddr,
    pub name: String,
    pub token: Option<String>,
    pub record_type: RecordType,
    pub answer: Option<IpAddr>,
    pub sequence: u64,
}

pub type EvidenceHook = Arc<dyn Fn(DnsEvidence) + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("invalid authoritative DNS zone")]
    InvalidZone,
    #[error("invalid DNS message: {0}")]
    Protocol(#[from] hickory_proto::ProtoError),
    #[error("invalid DNS message: {0}")]
    Decode(#[from] DecodeError),
    #[error("DNS I/O error: {0}")]
    Io(#[from] io::Error),
}

/// An authoritative UDP and TCP DNS server.
#[derive(Clone)]
pub struct DnsServer {
    config: DnsConfig,
    counts: Arc<Mutex<LruCache<String, u64>>>,
    tcp_limit: Arc<Semaphore>,
    evidence: Option<EvidenceHook>,
}

impl DnsServer {
    pub fn new(config: DnsConfig, evidence: Option<EvidenceHook>) -> Self {
        Self {
            config,
            counts: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(50_000).expect("nonzero DNS sequence capacity"),
            ))),
            tcp_limit: Arc::new(Semaphore::new(1_024)),
            evidence,
        }
    }

    /// Runs UDP and TCP listeners on the configured address until cancelled.
    pub async fn run(self) -> Result<(), DnsError> {
        let udp = UdpSocket::bind(self.config.udp_bind).await?;
        let tcp = TcpListener::bind(self.config.tcp_bind).await?;
        let udp_server = self.clone();
        let tcp_server = self;

        tokio::try_join!(udp_server.run_udp(udp), tcp_server.run_tcp(tcp))?;
        Ok(())
    }

    /// Runs both transports until a coordinated shutdown is requested.
    pub async fn run_until(self, mut shutdown: watch::Receiver<bool>) -> Result<(), DnsError> {
        let udp = UdpSocket::bind(self.config.udp_bind).await?;
        let tcp = TcpListener::bind(self.config.tcp_bind).await?;
        let udp_server = self.clone();
        let tcp_server = self;
        tokio::select! {
            result = udp_server.run_udp(udp) => result,
            result = tcp_server.run_tcp(tcp) => result,
            _ = shutdown.changed() => Ok(()),
        }
    }

    async fn run_udp(&self, socket: UdpSocket) -> Result<(), DnsError> {
        let mut packet = vec![0_u8; 65_535];
        loop {
            let (length, source) = socket.recv_from(&mut packet).await?;
            if let Ok(response) = self.respond(&packet[..length], source) {
                socket.send_to(&response, source).await?;
            }
        }
    }

    async fn run_tcp(&self, listener: TcpListener) -> Result<(), DnsError> {
        loop {
            let (stream, source) = listener.accept().await?;
            let server = self.clone();
            let Ok(permit) = server.tcp_limit.clone().try_acquire_owned() else {
                continue;
            };
            tokio::spawn(async move {
                let _permit = permit;
                match timeout(Duration::from_secs(30), server.handle_tcp(stream, source)).await {
                    Ok(Err(error)) => tracing::debug!(%source, %error, "DNS TCP connection closed"),
                    Err(_) => tracing::debug!(%source, "DNS TCP connection timed out"),
                    _ => {}
                }
            });
        }
    }

    async fn handle_tcp(&self, mut stream: TcpStream, source: SocketAddr) -> Result<(), DnsError> {
        loop {
            let length = match stream.read_u16().await {
                Ok(length) => usize::from(length),
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            let mut request = vec![0_u8; length];
            stream.read_exact(&mut request).await?;
            let response = self.respond(&request, source)?;
            let response_length = u16::try_from(response.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "DNS response too large")
            })?;
            stream.write_u16(response_length).await?;
            stream.write_all(&response).await?;
        }
    }

    fn respond(&self, packet: &[u8], source: SocketAddr) -> Result<Vec<u8>, DnsError> {
        let request = Message::from_bytes(packet)?;
        let sequence = request
            .queries
            .first()
            .filter(|query| {
                self.config.zone.zone_of(query.name())
                    && matches!(query.query_type(), RecordType::A | RecordType::AAAA)
                    && matches!(
                        resolve_name(query.name(), query.query_type(), &self.config, 0),
                        Resolution::Answer(_)
                    )
            })
            .map(|query| self.next_sequence(query.name(), query.query_type()))
            .unwrap_or(0);
        handle_packet(
            packet,
            source,
            &self.config,
            sequence,
            self.evidence.as_deref(),
        )
    }

    fn next_sequence(&self, name: &Name, record_type: RecordType) -> u64 {
        let key = format!("{}:{record_type}", name.to_ascii().to_lowercase());
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match counts.get_mut(&key) {
            Some(count) => {
                let sequence = *count;
                *count = count.saturating_add(1);
                sequence
            }
            None => {
                counts.put(key, 1);
                0
            }
        }
    }
}

/// Handles one DNS packet without hidden state. `sequence` is the zero-based
/// query number for this name, supplied by the caller.
pub fn handle_packet(
    packet: &[u8],
    source: SocketAddr,
    config: &DnsConfig,
    sequence: u64,
    evidence: Option<&(dyn Fn(DnsEvidence) + Send + Sync)>,
) -> Result<Vec<u8>, DnsError> {
    let request = Message::from_bytes(packet)?;
    let mut response = Message::response(request.metadata.id, request.metadata.op_code);
    response.metadata.recursion_desired = request.metadata.recursion_desired;
    response.metadata.recursion_available = false;

    if request.metadata.message_type != MessageType::Query
        || request.metadata.op_code != OpCode::Query
    {
        response.metadata.response_code = ResponseCode::NotImp;
        return Ok(response.to_bytes()?);
    }

    let Some(query) = request.queries.first() else {
        response.metadata.response_code = ResponseCode::FormErr;
        return Ok(response.to_bytes()?);
    };
    response.add_query(query.clone());

    if !config.zone.zone_of(query.name()) {
        response.metadata.authoritative = false;
        response.metadata.response_code = ResponseCode::Refused;
        return Ok(response.to_bytes()?);
    }
    response.metadata.authoritative = true;

    if query.name() == &config.zone {
        match query.query_type() {
            RecordType::SOA => {
                response.add_answer(soa_record(config));
                return Ok(response.to_bytes()?);
            }
            RecordType::NS => {
                response.add_answer(ns_record(config));
                return Ok(response.to_bytes()?);
            }
            _ => {}
        }
    }

    let record_type = query.query_type();
    let resolution = resolve_name(query.name(), record_type, config, sequence);
    let answer = match resolution {
        Resolution::Answer(ip) => {
            let rdata = match ip {
                IpAddr::V4(address) => RData::A(A(address)),
                IpAddr::V6(address) => RData::AAAA(AAAA(address)),
            };
            response.add_answer(Record::from_rdata(query.name().clone(), config.ttl, rdata));
            Some(ip)
        }
        Resolution::NoData => {
            response.add_authority(soa_record(config));
            None
        }
        Resolution::NameError => {
            response.metadata.response_code = ResponseCode::NXDomain;
            response.add_authority(soa_record(config));
            None
        }
    };

    if let Some(hook) = evidence {
        hook(DnsEvidence {
            source,
            name: query.name().to_ascii(),
            token: dns_token(query.name(), &config.zone),
            record_type,
            answer,
            sequence,
        });
    }

    Ok(response.to_bytes()?)
}

fn nameserver(config: &DnsConfig) -> Name {
    Name::from_ascii(format!("ns1.{}", config.zone.to_ascii()))
        .expect("configured zone produces a valid nameserver")
}

fn soa_record(config: &DnsConfig) -> Record {
    let hostmaster = Name::from_ascii(format!("hostmaster.{}", config.zone.to_ascii()))
        .expect("configured zone produces a valid mailbox name");
    Record::from_rdata(
        config.zone.clone(),
        config.ttl,
        RData::SOA(SOA::new(
            nameserver(config),
            hostmaster,
            1,
            300,
            60,
            86_400,
            config.ttl,
        )),
    )
}

fn ns_record(config: &DnsConfig) -> Record {
    Record::from_rdata(
        config.zone.clone(),
        config.ttl,
        RData::NS(NS(nameserver(config))),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    Answer(IpAddr),
    NoData,
    NameError,
}

fn resolve_name(
    name: &Name,
    record_type: RecordType,
    config: &DnsConfig,
    sequence: u64,
) -> Resolution {
    if !config.zone.zone_of(name) {
        return Resolution::NameError;
    }

    let fqdn = name.to_ascii().trim_end_matches('.').to_ascii_lowercase();
    let zone = config
        .zone
        .to_ascii()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let relative = fqdn
        .strip_suffix(&format!(".{zone}"))
        .or_else(|| (fqdn == zone).then_some(""));
    if relative == Some("") {
        return Resolution::NoData;
    }
    let Some(labels) = relative.map(|value| value.split('.').collect::<Vec<_>>()) else {
        return Resolution::NameError;
    };

    let pattern = if labels
        .first()
        .is_some_and(|label| label.starts_with("t-") && label.len() > 2)
    {
        &labels[1..]
    } else {
        labels.as_slice()
    };
    let selected = match pattern {
        ["ns1"] => Some(config.first_ip),
        ["static"] => Some(config.first_ip),
        ["alt"] => Some(if sequence.is_multiple_of(2) {
            config.first_ip
        } else {
            config.second_ip
        }),
        [count, "toctou"] => {
            let Ok(count) = count.parse::<u64>() else {
                return Resolution::NameError;
            };
            Some(if sequence < count {
                config.first_ip
            } else {
                config.second_ip
            })
        }
        [address, "static"] => decode_ip(address),
        [first, second, "alt"] => decode_ip(if sequence.is_multiple_of(2) {
            first
        } else {
            second
        }),
        [first, second, count, "toctou"] => {
            let Ok(count) = count.parse::<u64>() else {
                return Resolution::NameError;
            };
            decode_ip(if sequence < count { first } else { second })
        }
        _ => None,
    };

    let Some(ip) = selected else {
        return Resolution::NameError;
    };
    match (record_type, ip) {
        (RecordType::A, IpAddr::V4(_)) | (RecordType::AAAA, IpAddr::V6(_)) => {
            Resolution::Answer(ip)
        }
        (RecordType::A | RecordType::AAAA, _) => Resolution::NoData,
        _ => Resolution::NoData,
    }
}

fn dns_token(name: &Name, zone: &Name) -> Option<String> {
    let fqdn = name.to_ascii().trim_end_matches('.').to_ascii_lowercase();
    let zone = zone.to_ascii().trim_end_matches('.').to_ascii_lowercase();
    let relative = fqdn.strip_suffix(&format!(".{zone}"))?;
    relative
        .split('.')
        .next()
        .and_then(|label| label.strip_prefix("t-"))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

fn decode_ip(label: &str) -> Option<IpAddr> {
    if label.contains('-') {
        return Ipv4Addr::from_str(&label.replace('-', "."))
            .ok()
            .map(IpAddr::V4);
    }
    if label.len() == 32 && label.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let raw = u128::from_str_radix(label, 16).ok()?;
        return Some(IpAddr::V6(Ipv6Addr::from(raw)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::Query;
    use std::sync::Mutex;

    fn config() -> DnsConfig {
        DnsConfig::new("127.0.0.1:5353".parse().unwrap(), "rb.example.test.", 30).unwrap()
    }

    fn query(name: &str, record_type: RecordType) -> Vec<u8> {
        let mut message = Message::new(42, MessageType::Query, OpCode::Query);
        message.add_query(Query::query(Name::from_ascii(name).unwrap(), record_type));
        message.to_bytes().unwrap()
    }

    fn answer(packet: Vec<u8>, sequence: u64) -> Message {
        let response = handle_packet(
            &packet,
            "192.0.2.10:53000".parse().unwrap(),
            &config(),
            sequence,
            None,
        )
        .unwrap();
        Message::from_bytes(&response).unwrap()
    }

    #[test]
    fn static_ipv4_answer_is_authoritative() {
        let response = answer(query("127-0-0-1.static.rb.example.test.", RecordType::A), 9);
        assert!(response.metadata.authoritative);
        assert_eq!(response.metadata.response_code, ResponseCode::NoError);
        assert_eq!(&response.answers[0].data, &RData::A(A(Ipv4Addr::LOCALHOST)));
        assert_eq!(response.answers[0].ttl, 30);
    }

    #[test]
    fn alternating_answer_is_deterministic() {
        let packet = query("198-51-100-1.127-0-0-1.alt.rb.example.test.", RecordType::A);
        let first = answer(packet.clone(), 0);
        let second = answer(packet, 1);
        assert_eq!(
            &first.answers[0].data,
            &RData::A(A("198.51.100.1".parse().unwrap()))
        );
        assert_eq!(&second.answers[0].data, &RData::A(A(Ipv4Addr::LOCALHOST)));
    }

    #[test]
    fn toctou_switches_after_requested_count() {
        let packet = query(
            "192-0-2-1.169-254-169-254.2.toctou.rb.example.test.",
            RecordType::A,
        );
        assert_eq!(
            &answer(packet.clone(), 1).answers[0].data,
            &RData::A(A("192.0.2.1".parse().unwrap()))
        );
        assert_eq!(
            &answer(packet, 2).answers[0].data,
            &RData::A(A("169.254.169.254".parse().unwrap()))
        );
    }

    #[test]
    fn ipv6_and_nodata_are_supported() {
        let name = "00000000000000000000000000000001.static.rb.example.test.";
        let aaaa = answer(query(name, RecordType::AAAA), 0);
        let a = answer(query(name, RecordType::A), 0);
        assert_eq!(
            &aaaa.answers[0].data,
            &RData::AAAA(AAAA(Ipv6Addr::LOCALHOST))
        );
        assert!(a.answers.is_empty());
        assert_eq!(a.metadata.response_code, ResponseCode::NoError);
    }

    #[test]
    fn unknown_name_returns_nxdomain() {
        let response = answer(query("invalid.rb.example.test.", RecordType::A), 0);
        assert_eq!(response.metadata.response_code, ResponseCode::NXDomain);
        assert!(
            response
                .authorities
                .iter()
                .any(|record| record.record_type() == RecordType::SOA)
        );
    }

    #[test]
    fn serves_zone_authority_and_refuses_out_of_zone_names() {
        let soa = answer(query("rb.example.test.", RecordType::SOA), 0);
        let ns = answer(query("rb.example.test.", RecordType::NS), 0);
        assert_eq!(soa.answers[0].record_type(), RecordType::SOA);
        assert_eq!(ns.answers[0].record_type(), RecordType::NS);

        let apex_a = answer(query("rb.example.test.", RecordType::A), 0);
        assert_eq!(apex_a.metadata.response_code, ResponseCode::NoError);
        assert!(apex_a.answers.is_empty());
        assert_eq!(apex_a.authorities[0].record_type(), RecordType::SOA);

        let nameserver_a = answer(query("ns1.rb.example.test.", RecordType::A), 0);
        assert_eq!(
            &nameserver_a.answers[0].data,
            &RData::A(A("192.0.2.1".parse().unwrap()))
        );

        let outside = answer(query("static.other.example.", RecordType::A), 0);
        assert_eq!(outside.metadata.response_code, ResponseCode::Refused);
        assert!(!outside.metadata.authoritative);
    }

    #[test]
    fn emits_evidence() {
        let events = Mutex::new(Vec::new());
        let packet = query("t-case-7.127-0-0-1.static.rb.example.test.", RecordType::A);
        handle_packet(
            &packet,
            "192.0.2.10:53000".parse().unwrap(),
            &config(),
            3,
            Some(&|event| events.lock().unwrap().push(event)),
        )
        .unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].answer, Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert_eq!(events[0].token.as_deref(), Some("case-7"));
        assert_eq!(events[0].sequence, 3);
    }
}
