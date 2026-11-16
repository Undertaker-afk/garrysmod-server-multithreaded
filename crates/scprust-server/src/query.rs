//! Strict, single-datagram Source A2S query codec. Independently implemented
//! from the public wire format; provenance and intentional limits: docs/server.md.
pub const HEADER: [u8; 4] = [0xff; 4];
pub const INFO_PAYLOAD: &[u8] = b"Source Engine Query\0";
pub const MAX_REQUEST: usize = 29;
pub const MAX_REPLY: usize = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Info,
    Players,
    Rules,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub kind: Kind,
    pub challenge: Option<[u8; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Header,
    Length,
    Unsupported,
    Payload,
}

pub fn decode(packet: &[u8]) -> Result<Request, DecodeError> {
    if packet.len() < 5 || packet[..4] != HEADER {
        return Err(DecodeError::Header);
    }
    if packet.len() > MAX_REQUEST {
        return Err(DecodeError::Length);
    }
    match packet[4] {
        0x54 => {
            if packet.len() != 25 && packet.len() != 29 {
                return Err(DecodeError::Length);
            }
            if &packet[5..25] != INFO_PAYLOAD {
                return Err(DecodeError::Payload);
            }
            Ok(Request {
                kind: Kind::Info,
                challenge: if packet.len() == 29 {
                    Some([packet[25], packet[26], packet[27], packet[28]])
                } else {
                    None
                },
            })
        }
        code @ (0x55 | 0x56) => {
            if packet.len() != 9 {
                return Err(DecodeError::Length);
            }
            Ok(Request {
                kind: if code == 0x55 {
                    Kind::Players
                } else {
                    Kind::Rules
                },
                challenge: Some([packet[5], packet[6], packet[7], packet[8]]),
            })
        }
        _ => Err(DecodeError::Unsupported),
    }
}

pub fn challenge_reply(token: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(9);
    bytes.extend_from_slice(&HEADER);
    bytes.push(0x41);
    bytes.extend_from_slice(&token);
    bytes
}

#[derive(Debug)]
pub struct ServerInfo {
    name: String,
}

impl ServerInfo {
    pub fn new(label: &str) -> Result<Self, &'static str> {
        if label.is_empty() || label.len() > 64 || label.chars().any(char::is_control) {
            return Err("server label must be 1..64 UTF-8 bytes without control characters");
        }
        Ok(Self {
            name: format!("[QUERY ONLY] {label}"),
        })
    }

    pub fn response(&self, kind: Kind) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(&HEADER);
        match kind {
            Kind::Info => {
                bytes.extend_from_slice(&[0x49, 17]);
                cstring(&mut bytes, &self.name);
                cstring(&mut bytes, "no_map_loaded");
                cstring(&mut bytes, "garrysmod");
                cstring(&mut bytes, "Rust transport prototype (not playable)");
                bytes.extend_from_slice(&4000u16.to_le_bytes());
                // Players, slots and bots are all zero: no fake player capacity.
                // Linux dedicated, no password, no VAC/Steam authentication claim.
                bytes.extend_from_slice(&[0, 0, 0, b'd', b'l', 0, 0]);
                cstring(
                    &mut bytes,
                    concat!("scprust-query/", env!("CARGO_PKG_VERSION")),
                );
                bytes.push(0); // EDF: no optional fields.
            }
            Kind::Players => bytes.extend_from_slice(&[0x44, 0]),
            Kind::Rules => bytes.extend_from_slice(&[0x45, 0, 0]),
        }
        debug_assert!(bytes.len() <= MAX_REPLY);
        bytes
    }
}

fn cstring(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn info() -> Vec<u8> {
        [HEADER.as_slice(), &[0x54], INFO_PAYLOAD].concat()
    }
    #[test]
    fn accepts_only_complete_known_requests() {
        assert_eq!(
            decode(&info()).unwrap(),
            Request {
                kind: Kind::Info,
                challenge: None
            }
        );
        let mut challenged = info();
        challenged.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(decode(&challenged).unwrap().challenge, Some([1, 2, 3, 4]));
        for (code, kind) in [(0x55, Kind::Players), (0x56, Kind::Rules)] {
            let packet = [HEADER.as_slice(), &[code], &[0xff; 4]].concat();
            assert_eq!(
                decode(&packet).unwrap(),
                Request {
                    kind,
                    challenge: Some([0xff; 4])
                }
            );
            for length in 0..packet.len() {
                assert!(decode(&packet[..length]).is_err());
            }
        }
        for length in 0..25 {
            assert!(decode(&info()[..length]).is_err());
        }
        for length in [26, 27, 28, 30, 2048] {
            let mut invalid = info();
            invalid.resize(length, 0);
            assert!(decode(&invalid).is_err());
        }
        let mut invalid = info();
        invalid[24] = 1;
        assert_eq!(decode(&invalid), Err(DecodeError::Payload));
        assert!(decode(b"\xfe\xff\xff\xffTSource Engine Query\0").is_err());
        assert!(decode(b"\xff\xff\xff\xffgetchallenge\0").is_err());
    }
    #[test]
    fn golden_empty_responses_and_challenge_byte_order() {
        let info = ServerInfo::new("SCPRust").unwrap();
        assert_eq!(
            challenge_reply([1, 2, 3, 4]),
            b"\xff\xff\xff\xffA\x01\x02\x03\x04"
        );
        assert_eq!(info.response(Kind::Players), b"\xff\xff\xff\xffD\0");
        assert_eq!(info.response(Kind::Rules), b"\xff\xff\xff\xffE\0\0");
        let mut expected = b"\xff\xff\xff\xffI\x11[QUERY ONLY] SCPRust\0no_map_loaded\0garrysmod\0Rust transport prototype (not playable)\0\xa0\x0f\0\0\0dl\0\0".to_vec();
        expected.extend_from_slice(
            concat!("scprust-query/", env!("CARGO_PKG_VERSION"), "\0\0").as_bytes(),
        );
        assert_eq!(info.response(Kind::Info), expected);
        assert!(
            ServerInfo::new(&"x".repeat(64))
                .unwrap()
                .response(Kind::Info)
                .len()
                < MAX_REPLY
        );
    }
    #[test]
    fn untrusted_name_cannot_inject_fields_or_terminal_controls() {
        for label in ["", "hello\0map", "line\nbreak", "\u{1b}[31m"] {
            assert!(ServerInfo::new(label).is_err());
        }
        assert!(ServerInfo::new(&"x".repeat(65)).is_err());
        assert!(ServerInfo::new("Freunde – SCP").is_ok());
    }
    #[test]
    fn arbitrary_and_truncated_packets_never_panic() {
        let mut state = 0x12345678u32;
        for length in 0..2049 {
            let mut bytes = vec![0; length];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *byte = state as u8;
            }
            let _ = decode(&bytes);
            if length >= 4 {
                bytes[..4].copy_from_slice(&HEADER);
                let _ = decode(&bytes);
            }
        }
    }
}
