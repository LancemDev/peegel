//! Message types and framing. See docs/PROTOCOL.md.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 47820;
pub const SERVICE_TYPE: &str = "_peegel._tcp.local.";
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Pair,
    Resume,
}

/// Plain-text JSON frames exchanged before the session is encrypted.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Handshake {
    Hello {
        v: u32,
        mode: Mode,
        device_id: String,
        device_name: String,
        nonce: String,
    },
    Challenge {
        v: u32,
        server_id: String,
        server_name: String,
        nonce: String,
    },
    Error {
        code: String,
    },
}

/// Header of an encrypted frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Header {
    Auth,
    Welcome,
    Ping,
    Pong,
    Clip {
        id: String,
        mime: String,
        sha256: String,
        ts: u64,
    },
}

pub fn encode(header: &Header, body: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(header).expect("header serializes");
    let mut out = Vec::with_capacity(4 + json.len() + body.len());
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(body);
    out
}

pub fn decode(frame: &[u8]) -> Result<(Header, &[u8])> {
    if frame.len() < 4 {
        bail!("frame too short");
    }
    let len = u32::from_be_bytes(frame[..4].try_into()?) as usize;
    if frame.len() < 4 + len {
        bail!("header length exceeds frame");
    }
    let header = serde_json::from_slice(&frame[4..4 + len])?;
    Ok((header, &frame[4 + len..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let header = Header::Clip {
            id: "x".into(),
            mime: "text/plain".into(),
            sha256: "00".into(),
            ts: 5,
        };
        let frame = encode(&header, b"body");
        let (decoded, body) = decode(&frame).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(body, b"body");
    }

    #[test]
    fn handshake_json_shape() {
        let json = r#"{"t":"hello","v":1,"mode":"pair","device_id":"d","device_name":"n","nonce":"AA"}"#;
        let parsed: Handshake = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, Handshake::Hello { mode: Mode::Pair, .. }));
    }
}
