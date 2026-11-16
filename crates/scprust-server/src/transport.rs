//! Bounded connectionless query state, with an injectable monotonic clock and
//! entropy source so expiry, admission and source binding are deterministic tests.
use crate::query::{challenge_reply, decode, ServerInfo};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

const CHALLENGE_TTL: Duration = Duration::from_secs(30);
const MAX_CHALLENGES: usize = 1024;
const MAX_IPS: usize = 1024;
const IP_IDLE_TTL: u64 = 60;
const GLOBAL_PER_SECOND: u32 = 100;
const IP_PER_SECOND: u32 = 20;

pub trait Tokens {
    fn next_token(&mut self) -> io::Result<[u8; 4]>;
}

#[derive(Debug)]
struct Challenge {
    token: [u8; 4],
    expires: Duration,
}
#[derive(Debug)]
struct Window {
    second: u64,
    count: u32,
}
impl Window {
    fn new(second: u64) -> Self {
        Self { second, count: 0 }
    }
    fn reset(&mut self, second: u64) {
        if self.second != second {
            self.second = second;
            self.count = 0;
        }
    }
}

#[derive(Debug, Default)]
pub struct Counters {
    pub received: u64,
    pub invalid: u64,
    pub rate_limited: u64,
    pub cache_full: u64,
    pub challenges: u64,
    pub verified: u64,
}

pub struct QueryEngine {
    info: ServerInfo,
    challenges: HashMap<SocketAddr, Challenge>,
    ips: HashMap<IpAddr, Window>,
    global: Window,
    counters: Counters,
}

impl QueryEngine {
    pub fn new(info: ServerInfo) -> Self {
        Self {
            info,
            challenges: HashMap::new(),
            ips: HashMap::new(),
            global: Window::new(0),
            counters: Counters::default(),
        }
    }
    pub fn counters(&self) -> &Counters {
        &self.counters
    }

    /// Return no response for malformed/non-query traffic or exhausted limits.
    /// Tokens must come from OS entropy in production. The host passes elapsed
    /// monotonic time, not wall-clock timestamps. No requests are queued.
    pub fn handle(
        &mut self,
        peer: SocketAddr,
        packet: &[u8],
        now: Duration,
        tokens: &mut impl Tokens,
    ) -> io::Result<Option<Vec<u8>>> {
        self.counters.received = self.counters.received.saturating_add(1);
        let request = match decode(packet) {
            Ok(request) => request,
            Err(_) => {
                self.counters.invalid = self.counters.invalid.saturating_add(1);
                return Ok(None);
            }
        };
        if !self.admit(peer.ip(), now.as_secs()) {
            self.counters.rate_limited = self.counters.rate_limited.saturating_add(1);
            return Ok(None);
        }
        if let Some(challenge) = self.challenges.get(&peer) {
            if now < challenge.expires && request.challenge == Some(challenge.token) {
                self.counters.verified = self.counters.verified.saturating_add(1);
                return Ok(Some(self.info.response(request.kind)));
            }
        }
        // Invalid/absent/expired tokens never get a larger info/player/rules reply.
        // Reuse a live token to allow reordered query exchanges on one socket.
        let token = match self.challenges.get(&peer) {
            Some(challenge) if now < challenge.expires => challenge.token,
            _ => {
                self.challenges.retain(|_, c| now < c.expires);
                if self.challenges.len() >= MAX_CHALLENGES {
                    self.counters.cache_full = self.counters.cache_full.saturating_add(1);
                    return Ok(None);
                }
                let token = fresh_token(tokens)?;
                self.challenges.insert(
                    peer,
                    Challenge {
                        token,
                        expires: now.saturating_add(CHALLENGE_TTL),
                    },
                );
                token
            }
        };
        self.counters.challenges = self.counters.challenges.saturating_add(1);
        Ok(Some(challenge_reply(token)))
    }

    fn admit(&mut self, ip: IpAddr, second: u64) -> bool {
        self.global.reset(second);
        if self.global.count >= GLOBAL_PER_SECOND {
            return false;
        }
        // Bounded O(MAX_IPS) cleanup only when inserting a new source IP.
        if !self.ips.contains_key(&ip) {
            self.ips
                .retain(|_, w| second.saturating_sub(w.second) < IP_IDLE_TTL);
            if self.ips.len() >= MAX_IPS {
                return false;
            }
            self.ips.insert(ip, Window::new(second));
        }
        let window = self.ips.get_mut(&ip).expect("source window was inserted");
        window.reset(second);
        if window.count >= IP_PER_SECOND {
            return false;
        }
        window.count += 1;
        self.global.count += 1;
        true
    }
}

fn fresh_token(tokens: &mut impl Tokens) -> io::Result<[u8; 4]> {
    for _ in 0..8 {
        let token = tokens.next_token()?;
        // -1 is the traditional challenge request; zero is omitted by some clients.
        if token != [0xff; 4] && token != [0; 4] {
            return Ok(token);
        }
    }
    Err(io::Error::other(
        "entropy source produced reserved tokens repeatedly",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{HEADER, INFO_PAYLOAD};
    #[derive(Default)]
    struct Counter(u32);
    impl Tokens for Counter {
        fn next_token(&mut self) -> io::Result<[u8; 4]> {
            self.0 += 1;
            Ok(self.0.to_le_bytes())
        }
    }
    fn engine() -> QueryEngine {
        QueryEngine::new(ServerInfo::new("test").unwrap())
    }
    fn info(token: Option<&[u8]>) -> Vec<u8> {
        let mut packet = [HEADER.as_slice(), &[0x54], INFO_PAYLOAD].concat();
        if let Some(token) = token {
            packet.extend_from_slice(token);
        }
        packet
    }
    fn peer(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }
    fn ask(
        e: &mut QueryEngine,
        peer: SocketAddr,
        token: Option<&[u8]>,
        second: u64,
        source: &mut impl Tokens,
    ) -> Option<Vec<u8>> {
        e.handle(peer, &info(token), Duration::from_secs(second), source)
            .unwrap()
    }
    #[test]
    fn challenge_is_required_bound_to_address_and_expires() {
        let (mut e, mut source) = (engine(), Counter::default());
        let first = ask(&mut e, peer(1), None, 0, &mut source).unwrap();
        assert_eq!(&first[..5], b"\xff\xff\xff\xffA");
        let token = &first[5..];
        assert_eq!(
            ask(&mut e, peer(1), Some(token), 1, &mut source).unwrap()[4],
            0x49
        );
        assert_eq!(
            ask(&mut e, peer(2), Some(token), 1, &mut source).unwrap()[4],
            0x41
        );
        let other_ip = SocketAddr::from(([127, 0, 0, 2], 1));
        assert_eq!(
            ask(&mut e, other_ip, Some(token), 1, &mut source).unwrap()[4],
            0x41
        );
        let expired = ask(&mut e, peer(1), Some(token), 30, &mut source).unwrap();
        assert_eq!(expired[4], 0x41);
        assert_ne!(&expired[5..], token);
        assert_eq!(e.counters().verified, 1);
    }
    #[test]
    fn reordered_queries_reuse_token_and_empty_lists_require_it_too() {
        let (mut e, mut source) = (engine(), Counter::default());
        let first = ask(&mut e, peer(1), None, 0, &mut source).unwrap();
        assert_eq!(ask(&mut e, peer(1), None, 0, &mut source).unwrap(), first);
        for code in [0x55, 0x56] {
            let packet = [HEADER.as_slice(), &[code], &first[5..]].concat();
            assert_eq!(
                e.handle(peer(1), &packet, Duration::ZERO, &mut source)
                    .unwrap()
                    .unwrap()[4],
                code - 0x11
            );
            let initial = [HEADER.as_slice(), &[code], &[0xff; 4]].concat();
            assert_eq!(
                e.handle(peer(1), &initial, Duration::ZERO, &mut source)
                    .unwrap()
                    .unwrap()[4],
                0x41
            );
        }
    }
    #[test]
    fn per_ip_limit_cannot_be_bypassed_by_rotating_ports() {
        let (mut e, mut source) = (engine(), Counter::default());
        for port in 1..=20 {
            assert!(ask(&mut e, peer(port), None, 0, &mut source).is_some());
        }
        assert!(ask(&mut e, peer(999), None, 0, &mut source).is_none());
        assert!(ask(&mut e, peer(999), None, 1, &mut source).is_some());
        assert_eq!(e.counters().rate_limited, 1);
    }
    #[test]
    fn global_limit_and_bounded_ip_state() {
        let (mut e, mut source) = (engine(), Counter::default());
        for i in 0..100u8 {
            assert!(ask(
                &mut e,
                SocketAddr::from(([127, 0, 1, i], 1)),
                None,
                0,
                &mut source
            )
            .is_some());
        }
        assert!(ask(&mut e, peer(1), None, 0, &mut source).is_none());
        for i in 0..1024u16 {
            // Advance seconds to avoid the global limit, but not the 60s IP TTL.
            let addr = SocketAddr::from(([127, 1, (i / 256) as u8, i as u8], 1));
            let _ = ask(&mut e, addr, None, 1 + u64::from(i / 100), &mut source);
        }
        assert!(e.ips.len() <= MAX_IPS);
        assert!(e.challenges.len() <= MAX_CHALLENGES);
        assert!(ask(&mut e, peer(9999), None, 12, &mut source).is_none());
        assert!(ask(&mut e, peer(9999), None, 80, &mut source).is_some());
    }
    #[test]
    fn challenge_cache_is_bounded_independently_of_ip_limit() {
        let (mut e, mut source) = (engine(), Counter::default());
        for i in 0..1024u16 {
            let addr = SocketAddr::from(([127, 0, 0, (i % 100) as u8], i + 1));
            assert!(ask(&mut e, addr, None, u64::from(i / 100), &mut source).is_some());
        }
        assert_eq!(e.challenges.len(), MAX_CHALLENGES);
        assert!(ask(&mut e, peer(65000), None, 11, &mut source).is_none());
        assert_eq!(e.counters().cache_full, 1);
        assert!(ask(&mut e, peer(65000), None, 40, &mut source).is_some());
    }
    #[test]
    fn malformed_traffic_allocates_no_source_state() {
        let (mut e, mut source) = (engine(), Counter::default());
        for _ in 0..5000 {
            assert!(e
                .handle(peer(1), b"junk", Duration::ZERO, &mut source)
                .unwrap()
                .is_none());
        }
        assert!(e.ips.is_empty() && e.challenges.is_empty());
        assert_eq!(e.counters().invalid, 5000);
        assert!(ask(&mut e, peer(1), None, 0, &mut source).is_some());
    }
    #[test]
    fn entropy_errors_and_reserved_tokens_fail_closed() {
        struct Broken;
        impl Tokens for Broken {
            fn next_token(&mut self) -> io::Result<[u8; 4]> {
                Ok([0xff; 4])
            }
        }
        let mut e = engine();
        assert!(e
            .handle(peer(1), &info(None), Duration::ZERO, &mut Broken)
            .is_err());
        assert!(e.challenges.is_empty());
        struct Failed;
        impl Tokens for Failed {
            fn next_token(&mut self) -> io::Result<[u8; 4]> {
                Err(io::Error::other("test"))
            }
        }
        assert!(e
            .handle(peer(1), &info(None), Duration::ZERO, &mut Failed)
            .is_err());
    }
}
