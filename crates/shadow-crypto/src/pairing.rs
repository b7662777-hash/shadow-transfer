use std::time::{Duration, Instant};

use thiserror::Error;

pub const PAIRING_CODE_LEN: usize = 8;
/// Unambiguous alphabet: no O/0, I/1, L.
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const PROOF_CONTEXT: &[u8] = b"SHADOW-PAIR-v1";
const KEY_CONTEXT: &str = "shadow-transfer pairing v1";
pub const DEFAULT_PAIRING_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PairingError {
    #[error("failed to obtain secure randomness: {0}")]
    Random(String),
    #[error("pairing code must be {PAIRING_CODE_LEN} characters from the unambiguous alphabet")]
    InvalidCode,
    #[error("pairing code has expired")]
    Expired,
}

pub fn generate_pairing_code() -> Result<String, PairingError> {
    let mut code = String::with_capacity(PAIRING_CODE_LEN);
    while code.len() < PAIRING_CODE_LEN {
        let mut buf = [0u8; 32];
        getrandom::getrandom(&mut buf).map_err(|e| PairingError::Random(e.to_string()))?;
        for byte in buf {
            // Rejection sampling avoids modulo bias.
            if byte < 248 {
                code.push(ALPHABET[(byte % ALPHABET.len() as u8) as usize] as char);
                if code.len() == PAIRING_CODE_LEN {
                    break;
                }
            }
        }
    }
    Ok(code)
}

pub fn normalize_pairing_code(input: &str) -> Result<String, PairingError> {
    let mut normalized = String::with_capacity(PAIRING_CODE_LEN);
    for ch in input.chars() {
        if matches!(ch, ' ' | '-' | '_') {
            continue;
        }
        let upper = ch.to_ascii_uppercase();
        if !upper.is_ascii() || !ALPHABET.contains(&(upper as u8)) {
            return Err(PairingError::InvalidCode);
        }
        normalized.push(upper);
    }
    if normalized.len() != PAIRING_CODE_LEN {
        return Err(PairingError::InvalidCode);
    }
    Ok(normalized)
}

pub fn pairing_key(code: &str) -> Result<[u8; 32], PairingError> {
    let normalized = normalize_pairing_code(code)?;
    Ok(blake3::derive_key(KEY_CONTEXT, normalized.as_bytes()))
}

/// Proof that the client knows the code, bound to both identities and both
/// nonces. Message order is client fingerprint, then server fingerprint.
pub fn pairing_proof(
    code: &str,
    client_fingerprint: &[u8; 32],
    server_fingerprint: &[u8; 32],
    client_nonce: &[u8; 16],
    server_nonce: &[u8; 16],
) -> Result<[u8; 32], PairingError> {
    let key = pairing_key(code)?;
    Ok(proof_with_key(
        &key,
        client_fingerprint,
        server_fingerprint,
        client_nonce,
        server_nonce,
    ))
}

fn proof_with_key(
    key: &[u8; 32],
    client_fingerprint: &[u8; 32],
    server_fingerprint: &[u8; 32],
    client_nonce: &[u8; 16],
    server_nonce: &[u8; 16],
) -> [u8; 32] {
    let mut input = Vec::with_capacity(PROOF_CONTEXT.len() + 96);
    input.extend_from_slice(PROOF_CONTEXT);
    input.extend_from_slice(client_fingerprint);
    input.extend_from_slice(server_fingerprint);
    input.extend_from_slice(client_nonce);
    input.extend_from_slice(server_nonce);
    *blake3::keyed_hash(key, &input).as_bytes()
}

pub fn verify_pairing_proof(
    code: &str,
    client_fingerprint: &[u8; 32],
    server_fingerprint: &[u8; 32],
    client_nonce: &[u8; 16],
    server_nonce: &[u8; 16],
    proof_hex: &str,
) -> bool {
    let Ok(key) = pairing_key(code) else {
        return false;
    };
    let Ok(proof) = hex::decode(proof_hex) else {
        return false;
    };
    if proof.len() != 32 {
        return false;
    }
    let expected = proof_with_key(
        &key,
        client_fingerprint,
        server_fingerprint,
        client_nonce,
        server_nonce,
    );
    crate::constant_time_eq(&proof, &expected)
}

/// In-memory pairing state for a receiving device. Codes are single-use,
/// short-lived and rotated by the receiver.
#[derive(Debug)]
pub struct PairingSession {
    code: String,
    created: Instant,
    ttl: Duration,
}

impl PairingSession {
    pub fn new(ttl: Duration) -> Result<Self, PairingError> {
        let mut session = Self {
            code: generate_pairing_code()?,
            created: Instant::now(),
            ttl,
        };
        session.ensure_fresh()?;
        Ok(session)
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn is_expired(&self) -> bool {
        self.created.elapsed() >= self.ttl
    }

    pub fn remaining_secs(&self) -> u32 {
        self.ttl.saturating_sub(self.created.elapsed()).as_secs() as u32
    }

    pub fn ensure_fresh(&mut self) -> Result<bool, PairingError> {
        if self.is_expired() {
            self.code = generate_pairing_code()?;
            self.created = Instant::now();
            return Ok(true);
        }
        Ok(false)
    }

    pub fn rotate(&mut self) -> Result<(), PairingError> {
        self.code = generate_pairing_code()?;
        self.created = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_long_and_unambiguous() {
        for _ in 0..200 {
            let code = generate_pairing_code().unwrap();
            assert_eq!(code.len(), PAIRING_CODE_LEN);
            assert!(code.bytes().all(|b| ALPHABET.contains(&b)));
        }
    }

    #[test]
    fn normalization_accepts_user_formatting() {
        assert_eq!(normalize_pairing_code("abcd-efgh").unwrap(), "ABCDEFGH");
        assert_eq!(normalize_pairing_code("ABCD EFGH").unwrap(), "ABCDEFGH");
        assert!(normalize_pairing_code("ABCD-EFG").is_err());
        assert!(normalize_pairing_code("ABCD-EFG0").is_err());
    }

    #[test]
    fn proof_binds_both_identities_and_nonces() {
        let code = "ABCD2345";
        let client_fp = [1u8; 32];
        let server_fp = [2u8; 32];
        let client_nonce = [3u8; 16];
        let server_nonce = [4u8; 16];
        let proof =
            pairing_proof(code, &client_fp, &server_fp, &client_nonce, &server_nonce).unwrap();
        let proof_hex = hex::encode(proof);
        assert!(verify_pairing_proof(
            "abcd-2345",
            &client_fp,
            &server_fp,
            &client_nonce,
            &server_nonce,
            &proof_hex
        ));
        assert!(!verify_pairing_proof(
            "ABCD2345",
            &server_fp,
            &client_fp,
            &client_nonce,
            &server_nonce,
            &proof_hex
        ));
        assert!(!verify_pairing_proof(
            "ABCD2345",
            &client_fp,
            &server_fp,
            &[9u8; 16],
            &server_nonce,
            &proof_hex
        ));
        assert!(!verify_pairing_proof(
            "WRONG234",
            &client_fp,
            &server_fp,
            &client_nonce,
            &server_nonce,
            &proof_hex
        ));
        assert!(!verify_pairing_proof(
            "ABCD2345",
            &client_fp,
            &server_fp,
            &client_nonce,
            &server_nonce,
            "bogus"
        ));
    }

    #[test]
    fn session_rotates_when_expired() {
        let mut session = PairingSession::new(Duration::from_millis(1)).unwrap();
        let first = session.code().to_string();
        std::thread::sleep(Duration::from_millis(5));
        assert!(session.is_expired());
        assert!(session.ensure_fresh().unwrap());
        assert_ne!(first, session.code());
    }
}
