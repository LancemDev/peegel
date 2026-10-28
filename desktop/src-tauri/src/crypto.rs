//! Key derivation and frame encryption. See docs/PROTOCOL.md.

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, Key, KeyInit, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

pub fn b64(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

pub fn unb64(data: &str) -> Result<Vec<u8>> {
    Ok(URL_SAFE_NO_PAD.decode(data)?)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn derive_device_key(pair_secret: &[u8], device_id: &str) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(device_id.as_bytes()), pair_secret);
    let mut out = [0u8; 32];
    hk.expand(b"peegel/v1/device-key", &mut out)
        .expect("32 bytes is a valid HKDF output length");
    out
}

/// Returns `(client_to_server, server_to_client)` keys.
pub fn derive_session_keys(
    device_key: &[u8],
    client_nonce: &[u8],
    server_nonce: &[u8],
) -> ([u8; 32], [u8; 32]) {
    let salt = [client_nonce, server_nonce].concat();
    let hk = Hkdf::<Sha256>::new(Some(&salt), device_key);
    let mut okm = [0u8; 64];
    hk.expand(b"peegel/v1/session", &mut okm)
        .expect("64 bytes is a valid HKDF output length");
    let mut c2s = [0u8; 32];
    let mut s2c = [0u8; 32];
    c2s.copy_from_slice(&okm[..32]);
    s2c.copy_from_slice(&okm[32..]);
    (c2s, s2c)
}

fn counter_nonce(counter: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

/// One direction of an encrypted session with an implicit counter nonce.
pub struct FrameCipher {
    cipher: ChaCha20Poly1305,
    counter: u64,
}

impl FrameCipher {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(key)),
            counter: 0,
        }
    }

    pub fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let nonce = counter_nonce(self.counter);
        self.counter += 1;
        self.cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory buffers")
    }

    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let nonce = counter_nonce(self.counter);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext)
            .map_err(|_| anyhow!("frame failed to decrypt"))?;
        self.counter += 1;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shared with android/app/src/test/.../CryptoTest.kt so both sides stay compatible.
    #[test]
    fn known_vectors() {
        let device_key = derive_device_key(&[7u8; 32], "phone-1");
        assert_eq!(
            hex::encode(device_key),
            "3efd567dc00a006d30bd088a7e172909cebf196d08f957999649a963c2db57a4"
        );
        let (c2s, s2c) = derive_session_keys(&device_key, &[1u8; 32], &[2u8; 32]);
        assert_eq!(
            hex::encode(c2s),
            "9ba6af2735b49a685654595352a9d7b13d6967023ed00e8e6a23cf92a97d2661"
        );
        assert_eq!(
            hex::encode(s2c),
            "db0b68ed8c23fb6321f55c69ea4b3fb495b0a41cca1e08d2a0c916b9a8c96e75"
        );
        let frame = crate::protocol::encode(&crate::protocol::Header::Auth, &[]);
        assert_eq!(
            hex::encode(FrameCipher::new(&c2s).seal(&frame)),
            "0f7dcdd99b16f4ec3665ed880aceb93533775b9a5ce751fdfca2d3b92bc5adb2"
        );
    }

    #[test]
    fn roundtrip_and_counter() {
        let key = [1u8; 32];
        let mut a = FrameCipher::new(&key);
        let mut b = FrameCipher::new(&key);
        let first = a.seal(b"hello");
        let second = a.seal(b"world");
        assert_eq!(b.open(&first).unwrap(), b"hello");
        assert_eq!(b.open(&second).unwrap(), b"world");
        assert!(b.open(&first).is_err(), "replayed frame must be rejected");
    }
}
