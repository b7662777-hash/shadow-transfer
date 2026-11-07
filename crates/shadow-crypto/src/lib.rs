//! Security primitives for Shadow Transfer.
//!
//! Security model:
//! - Every device owns a persistent ECDSA P-256 identity and a self-signed
//!   certificate. The certificate fingerprint (SHA-256) *is* the device identity.
//! - Connections are TLS 1.3 only and mutual-authentication is mandatory.
//!   rustls validates the handshake signature; trust is decided by exact
//!   certificate-fingerprint pinning in the application layer, before any
//!   application message is processed.
//! - A device we have never seen can only become trusted through an
//!   out-of-band pairing code (about 40 bits of entropy) plus an explicit
//!   accept action on the receiving device. The proof binds both certificate
//!   fingerprints and both nonces, preventing relays and replay.
//! - Trust stores are strict: corrupt trust data is an error, never a reset.

mod identity;
mod pairing;
mod tls;
mod trust;

pub use identity::{Identity, IdentityError};
pub use pairing::{
    generate_pairing_code, normalize_pairing_code, pairing_key, pairing_proof,
    verify_pairing_proof, PairingError, PairingSession, DEFAULT_PAIRING_TTL, PAIRING_CODE_LEN,
};
pub use tls::{client_config, server_config, TlsConfigError, ALPN_PROTOCOL};
pub use trust::{unix_now, TrustStore, TrustStoreError, TrustedDevice};

pub use rustls;

/// SHA-256 fingerprint of a DER certificate.
pub fn fingerprint(cert_der: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(cert_der);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn fingerprint_hex(cert_der: &[u8]) -> String {
    hex::encode(fingerprint(cert_der))
}

/// Constant-time comparison for secret-derived byte strings.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
