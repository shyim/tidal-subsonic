//! Encrypt secrets (TIDAL tokens) at rest. In multi-user mode a single SQLite
//! file holds every linked user's TIDAL session, so a leaked DB must not hand an
//! attacker usable tokens.
//!
//! The master key comes from `TIDAL_SUBSONIC_KEY` (base64, 32 bytes) if set,
//! otherwise a key is generated once and persisted in the `config` table. That
//! keeps the key next to the data (not ideal, but standard for self-hosted) and
//! is strictly better than storing tokens in plaintext. Set the env var to keep
//! the key out of the DB entirely.

use base64::Engine;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use rand::RngCore;

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

#[derive(Clone)]
pub struct Cipher {
    key: [u8; 32],
}

impl Cipher {
    pub fn new(key: [u8; 32]) -> Self {
        Cipher { key }
    }

    /// Generate a fresh random 32-byte key.
    pub fn generate_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        key
    }

    /// Parse a base64-encoded 32-byte key (e.g. from an env var).
    pub fn key_from_base64(s: &str) -> Option<[u8; 32]> {
        let bytes = B64.decode(s.trim()).ok()?;
        bytes.try_into().ok()
    }

    pub fn key_to_base64(key: &[u8; 32]) -> String {
        B64.encode(key)
    }

    /// Encrypt `plaintext`, returning base64(nonce ‖ ciphertext). Empty input
    /// encrypts to empty (so an unset token stays visibly unset).
    pub fn encrypt(&self, plaintext: &str) -> String {
        if plaintext.is_empty() {
            return String::new();
        }
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        match cipher.encrypt(nonce, plaintext.as_bytes()) {
            Ok(ct) => {
                let mut out = Vec::with_capacity(12 + ct.len());
                out.extend_from_slice(&nonce_bytes);
                out.extend_from_slice(&ct);
                B64.encode(out)
            }
            // Encryption of a valid key never fails in practice; on the
            // impossible error, refuse to persist a bogus value.
            Err(_) => String::new(),
        }
    }

    /// Decrypt a base64(nonce ‖ ciphertext) value produced by `encrypt`. Returns
    /// the plaintext, or empty on empty input / any failure (tampered, wrong key).
    pub fn decrypt(&self, encoded: &str) -> String {
        if encoded.is_empty() {
            return String::new();
        }
        let Ok(raw) = B64.decode(encoded.trim()) else {
            return String::new();
        };
        if raw.len() < 12 {
            return String::new();
        }
        let (nonce_bytes, ct) = raw.split_at(12);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ct)
            .ok()
            .and_then(|pt| String::from_utf8(pt).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = Cipher::new(Cipher::generate_key());
        let secret = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9.some.tidal.token";
        let enc = c.encrypt(secret);
        assert_ne!(enc, secret);
        assert!(!enc.is_empty());
        assert_eq!(c.decrypt(&enc), secret);
    }

    #[test]
    fn empty_stays_empty() {
        let c = Cipher::new(Cipher::generate_key());
        assert_eq!(c.encrypt(""), "");
        assert_eq!(c.decrypt(""), "");
    }

    #[test]
    fn wrong_key_fails_closed() {
        let a = Cipher::new(Cipher::generate_key());
        let b = Cipher::new(Cipher::generate_key());
        let enc = a.encrypt("secret");
        assert_eq!(b.decrypt(&enc), ""); // AEAD auth failure → empty, not garbage
    }

    #[test]
    fn base64_key_roundtrip() {
        let key = Cipher::generate_key();
        let s = Cipher::key_to_base64(&key);
        assert_eq!(Cipher::key_from_base64(&s), Some(key));
    }
}
