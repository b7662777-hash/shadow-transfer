use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::CertificateDer;
use thiserror::Error;

use crate::fingerprint;

const CERT_FILE: &str = "identity.cert.der";
const KEY_FILE: &str = "identity.key.der";

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("certificate generation failed: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("invalid stored identity: {0}")]
    Invalid(String),
}

/// Persistent device identity. The private key never leaves the device.
#[derive(Clone)]
pub struct Identity {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
    fingerprint: [u8; 32],
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &hex::encode(self.fingerprint))
            .finish_non_exhaustive()
    }
}

impl Identity {
    pub fn generate() -> Result<Self, IdentityError> {
        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::new(vec!["shadow-transfer.invalid".to_string()])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Shadow Transfer Device");
        dn.push(DnType::OrganizationName, "Shadow Transfer");
        params.distinguished_name = dn;
        params.is_ca = rcgen::IsCa::ExplicitNoCa;
        let cert = params.self_signed(&key_pair)?;
        let cert_der = cert.der().to_vec();
        let key_der = key_pair.serialize_der();
        let fp = fingerprint(&cert_der);
        Ok(Self {
            cert_der,
            key_der,
            fingerprint: fp,
        })
    }

    pub fn load_or_create(dir: &Path) -> Result<Self, IdentityError> {
        let cert_path = dir.join(CERT_FILE);
        let key_path = dir.join(KEY_FILE);
        if cert_path.exists() && key_path.exists() {
            return Self::load(&cert_path, &key_path);
        }
        fs::create_dir_all(dir)?;
        let identity = Self::generate()?;
        identity.save(dir)?;
        Ok(identity)
    }

    pub fn load(cert_path: &Path, key_path: &Path) -> Result<Self, IdentityError> {
        let cert_der = fs::read(cert_path)?;
        let key_der = fs::read(key_path)?;
        if cert_der.is_empty() || key_der.is_empty() {
            return Err(IdentityError::Invalid("empty identity file".into()));
        }
        let fp = fingerprint(&cert_der);
        Ok(Self {
            cert_der,
            key_der,
            fingerprint: fp,
        })
    }

    pub fn save(&self, dir: &Path) -> Result<(), IdentityError> {
        fs::create_dir_all(dir)?;
        write_private(&dir.join(KEY_FILE), &self.key_der)?;
        write_private(&dir.join(CERT_FILE), &self.cert_der)?;
        Ok(())
    }

    pub fn cert_der(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    pub fn cert_der_bytes(&self) -> &[u8] {
        &self.cert_der
    }

    pub fn key_der(&self) -> &[u8] {
        &self.key_der
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    pub fn fingerprint_hex(&self) -> String {
        hex::encode(self.fingerprint)
    }
}

fn write_private(path: &PathBuf, bytes: &[u8]) -> Result<(), IdentityError> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_persists_and_keeps_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let first = Identity::load_or_create(dir.path()).unwrap();
        let second = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(first.cert_der_bytes(), second.cert_der_bytes());
        assert_eq!(first.key_der(), second.key_der());
    }

    #[test]
    fn generated_identities_are_unique() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn empty_identity_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join("c");
        let key = dir.path().join("k");
        fs::write(&cert, []).unwrap();
        fs::write(&key, []).unwrap();
        assert!(matches!(
            Identity::load(&cert, &key),
            Err(IdentityError::Invalid(_))
        ));
    }
}
