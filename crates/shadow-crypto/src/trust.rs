use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrustStoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("trust store is corrupt: {0}")]
    Corrupt(String),
    #[error("trust store lock poisoned")]
    Poisoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedDevice {
    pub name: String,
    pub fingerprint: String,
    pub added_at: u64,
    pub last_seen: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustFile {
    version: u32,
    devices: Vec<TrustedDevice>,
}

const TRUST_VERSION: u32 = 1;

/// A small, strictly validated pin store. Each entry is a device certificate
/// fingerprint that the user explicitly approved during pairing.
pub struct TrustStore {
    path: PathBuf,
    entries: Mutex<Vec<TrustedDevice>>,
}

impl std::fmt::Debug for TrustStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustStore")
            .field("path", &self.path)
            .field(
                "devices",
                &self.entries.lock().map(|e| e.len()).unwrap_or_default(),
            )
            .finish()
    }
}

impl TrustStore {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, TrustStoreError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                entries: Mutex::new(Vec::new()),
            });
        }
        let bytes = fs::read(&path)?;
        let file: TrustFile =
            serde_json::from_slice(&bytes).map_err(|e| TrustStoreError::Corrupt(e.to_string()))?;
        if file.version != TRUST_VERSION {
            return Err(TrustStoreError::Corrupt(format!(
                "unsupported trust store version {}",
                file.version
            )));
        }
        for device in &file.devices {
            if !is_fingerprint_hex(&device.fingerprint) {
                return Err(TrustStoreError::Corrupt(format!(
                    "invalid fingerprint for {}",
                    device.name
                )));
            }
        }
        Ok(Self {
            path,
            entries: Mutex::new(file.devices),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> Result<Vec<TrustedDevice>, TrustStoreError> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| TrustStoreError::Poisoned)?
            .clone())
    }

    pub fn is_trusted(&self, fingerprint: &[u8; 32]) -> bool {
        let hex = hex::encode(fingerprint);
        self.entries
            .lock()
            .map(|entries| entries.iter().any(|d| d.fingerprint == hex))
            .unwrap_or(false)
    }

    /// Adds a device if unknown. Returns true when the store changed.
    pub fn trust(
        &self,
        name: &str,
        fingerprint: &[u8; 32],
        now: u64,
    ) -> Result<bool, TrustStoreError> {
        let hex = hex::encode(fingerprint);
        let mut guard = self.entries.lock().map_err(|_| TrustStoreError::Poisoned)?;
        if let Some(existing) = guard.iter_mut().find(|d| d.fingerprint == hex) {
            if existing.last_seen != now {
                existing.last_seen = now;
                let snapshot = guard.clone();
                Self::persist(&self.path, &snapshot)?;
            }
            return Ok(false);
        }
        let mut next = guard.clone();
        next.push(TrustedDevice {
            name: sanitize_name(name),
            fingerprint: hex,
            added_at: now,
            last_seen: now,
        });
        Self::persist(&self.path, &next)?;
        *guard = next;
        Ok(true)
    }

    pub fn touch(&self, fingerprint: &[u8; 32], now: u64) -> Result<(), TrustStoreError> {
        let hex = hex::encode(fingerprint);
        let mut guard = self.entries.lock().map_err(|_| TrustStoreError::Poisoned)?;
        if let Some(existing) = guard.iter_mut().find(|d| d.fingerprint == hex) {
            if now.saturating_sub(existing.last_seen) < 60 {
                return Ok(());
            }
            existing.last_seen = now;
            let snapshot = guard.clone();
            Self::persist(&self.path, &snapshot)?;
        }
        Ok(())
    }

    pub fn revoke(&self, fingerprint: &[u8; 32]) -> Result<bool, TrustStoreError> {
        let hex = hex::encode(fingerprint);
        let mut guard = self.entries.lock().map_err(|_| TrustStoreError::Poisoned)?;
        let next: Vec<_> = guard
            .iter()
            .filter(|d| d.fingerprint != hex)
            .cloned()
            .collect();
        let changed = next.len() != guard.len();
        if changed {
            Self::persist(&self.path, &next)?;
            *guard = next;
        }
        Ok(changed)
    }

    fn persist(path: &Path, devices: &[TrustedDevice]) -> Result<(), TrustStoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = TrustFile {
            version: TRUST_VERSION,
            devices: devices.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|e| TrustStoreError::Corrupt(e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut handle = fs::File::create(&tmp)?;
            handle.write_all(&bytes)?;
            handle.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

fn is_fingerprint_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn sanitize_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Unknown device".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_store_roundtrip_and_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted.json");
        let store = TrustStore::load(&path).unwrap();
        let fp = [5u8; 32];
        assert!(!store.is_trusted(&fp));
        assert!(store.trust("Tablet", &fp, 100).unwrap());
        assert!(store.is_trusted(&fp));
        assert!(!store.trust("Tablet", &fp, 101).unwrap());

        let reloaded = TrustStore::load(&path).unwrap();
        assert!(reloaded.is_trusted(&fp));
        assert_eq!(reloaded.list().unwrap()[0].name, "Tablet");

        assert!(reloaded.revoke(&fp).unwrap());
        assert!(!TrustStore::load(&path).unwrap().is_trusted(&fp));
    }

    #[test]
    fn corrupt_store_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted.json");
        fs::write(&path, b"{\"version\":1,\"devices\":[{\"name\":\"x\"}]}").unwrap();
        assert!(matches!(
            TrustStore::load(&path),
            Err(TrustStoreError::Corrupt(_))
        ));
    }

    #[test]
    fn invalid_fingerprint_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted.json");
        fs::write(
            &path,
            br#"{"version":1,"devices":[{"name":"x","fingerprint":"zz","added_at":0,"last_seen":0}]}"#,
        )
        .unwrap();
        assert!(matches!(
            TrustStore::load(&path),
            Err(TrustStoreError::Corrupt(_))
        ));
    }
}
