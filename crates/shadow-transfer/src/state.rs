use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use blake3::Hash;
use serde::{Deserialize, Serialize};
use shadow_protocol::{validate_transfer_id, FileMeta, ResumeEntry, TransferRequest};
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tracing::{info, warn};

use crate::error::TransferError;
use crate::path::{reserve_unique, verify_wire_path, wire_path_below};

const STATE_DIR_NAME: &str = ".shadow-transfer-state";
const MANIFEST_VERSION: u32 = 1;
const HASH_BUFFER: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    transfer_id: String,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    file_id: u32,
    rel_path: String,
    size: u64,
    modified_ms: i64,
    completed: bool,
    #[serde(default)]
    final_path: Option<String>,
    #[serde(default)]
    hash: Option<String>,
}

struct ActiveFile {
    file_id: u32,
    file: File,
    next_offset: u64,
    /// Incremental full-file hash for a file started at offset 0. A resumed
    /// file has no hasher and is re-read before finalization.
    hasher: Option<blake3::Hasher>,
}

#[derive(Debug, Clone)]
pub struct FinalizedFile {
    pub destination: PathBuf,
    pub rel_path: String,
    pub bytes: u64,
}

/// Per-transfer receiving state: partial file, manifest and resume verification.
pub struct ReceiveState {
    dest_root: PathBuf,
    state_dir: PathBuf,
    transfer_id: String,
    manifest: Manifest,
    initial_completed: HashSet<u32>,
    session_completed: HashSet<u32>,
    entries: Vec<ResumeEntry>,
    active: Option<ActiveFile>,
}

impl ReceiveState {
    pub async fn open(
        dest_root: PathBuf,
        request: &TransferRequest,
    ) -> Result<Self, TransferError> {
        validate_transfer_id(&request.transfer_id)
            .map_err(|e| TransferError::State(e.to_string()))?;
        request.validate().map_err(TransferError::Validation)?;
        tokio::fs::create_dir_all(&dest_root).await?;
        let state_root = dest_root.join(STATE_DIR_NAME);
        let state_dir = state_root.join(&request.transfer_id);
        tokio::fs::create_dir_all(&state_dir).await?;

        let manifest_path = state_dir.join("manifest.json");
        let mut manifest = match load_manifest(&manifest_path) {
            Ok(Some(existing)) if manifest_matches(&existing, request) => existing,
            Ok(Some(_)) => {
                warn!("transfer state does not match request; resetting partial data");
                reset_state_dir(&state_dir).await?;
                fresh_manifest(request)
            }
            Ok(None) => fresh_manifest(request),
            Err(err) => {
                warn!(error = %err, "corrupt receive manifest; resetting partial data");
                reset_state_dir(&state_dir).await?;
                fresh_manifest(request)
            }
        };

        let mut initial_completed = HashSet::new();
        let mut entries = Vec::with_capacity(manifest.files.len());

        for index in 0..manifest.files.len() {
            let needs_completion_check =
                manifest.files[index].completed && manifest.files[index].hash.is_some();
            if needs_completion_check {
                let completed = verify_completed_file(&dest_root, &manifest.files[index]).await;
                match completed {
                    Ok(true) => {
                        let file = &manifest.files[index];
                        initial_completed.insert(file.file_id);
                        entries.push(ResumeEntry {
                            file_id: file.file_id,
                            offset: file.size,
                            prefix_hash: file.hash.clone().unwrap_or_default(),
                            completed: true,
                        });
                        continue;
                    }
                    Ok(false) => {
                        warn!(
                            path = %manifest.files[index].rel_path,
                            "previously received file is missing or changed; it will be resent"
                        );
                        manifest.files[index].completed = false;
                        manifest.files[index].final_path = None;
                        manifest.files[index].hash = None;
                    }
                    Err(err) => {
                        warn!(error = %err, "failed to revalidate completed file");
                        manifest.files[index].completed = false;
                        manifest.files[index].final_path = None;
                        manifest.files[index].hash = None;
                    }
                }
            }

            let file = &manifest.files[index];
            if file.completed && (file.hash.is_none() || file.final_path.is_none()) {
                manifest.files[index].completed = false;
            }
            let file = &manifest.files[index];
            let part_path = part_path(&state_dir, file.file_id);
            let (offset, prefix_hash) = match tokio::fs::metadata(&part_path).await {
                Ok(meta) if meta.is_file() => {
                    let len = meta.len();
                    // Never offer an offset equal to the file size for a file
                    // that is not finalized: the sender would skip it and the
                    // receiver would never complete verification.
                    if len == 0 || len >= file.size {
                        (0, blake3::hash(&[]))
                    } else {
                        let hash = hash_prefix(part_path.clone(), len).await?;
                        (len, hash)
                    }
                }
                _ => (0, blake3::hash(&[])),
            };
            entries.push(ResumeEntry {
                file_id: file.file_id,
                offset,
                prefix_hash: prefix_hash.to_hex().to_string(),
                completed: false,
            });
        }

        save_manifest(&state_dir, &manifest).await?;

        Ok(Self {
            dest_root,
            state_dir,
            transfer_id: request.transfer_id.clone(),
            manifest,
            initial_completed,
            session_completed: HashSet::new(),
            entries,
            active: None,
        })
    }

    pub fn resume_entries(&self) -> Vec<ResumeEntry> {
        self.entries.clone()
    }

    fn file(&self, file_id: u32) -> Result<&ManifestFile, TransferError> {
        self.manifest
            .files
            .iter()
            .find(|f| f.file_id == file_id)
            .ok_or_else(|| TransferError::State(format!("unknown file id {file_id}")))
    }

    pub async fn begin_file(&mut self, file_id: u32, offset: u64) -> Result<(), TransferError> {
        if self.active.is_some() {
            return Err(TransferError::State(
                "a file is already being written".into(),
            ));
        }
        let size = self.file(file_id)?.size;
        if offset > size {
            return Err(TransferError::State(
                "resume offset exceeds file size".into(),
            ));
        }
        let entry = self
            .manifest
            .files
            .iter_mut()
            .find(|f| f.file_id == file_id)
            .ok_or_else(|| TransferError::State(format!("unknown file id {file_id}")))?;
        if entry.completed {
            // The sender decided to resend this file: keep the old finalized copy
            // (never delete user-visible data) and receive a new one.
            entry.completed = false;
            entry.final_path = None;
            entry.hash = None;
            save_manifest(&self.state_dir, &self.manifest).await?;
        }

        let path = part_path(&self.state_dir, file_id);
        let mut file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .await?;
        file.set_len(offset).await?;
        file.seek(SeekFrom::Start(offset)).await?;
        let hasher = if offset == 0 {
            Some(blake3::Hasher::new())
        } else {
            None
        };
        self.active = Some(ActiveFile {
            file_id,
            file,
            next_offset: offset,
            hasher,
        });
        Ok(())
    }

    pub async fn write_chunk(
        &mut self,
        file_id: u32,
        offset: u64,
        data: &[u8],
    ) -> Result<(), TransferError> {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| TransferError::State("no active file".into()))?;
        if active.file_id != file_id {
            return Err(TransferError::State("chunk for unexpected file".into()));
        }
        if offset != active.next_offset {
            return Err(TransferError::State(format!(
                "non-contiguous chunk: expected offset {}, got {offset}",
                active.next_offset
            )));
        }
        active.file.write_all(data).await?;
        if let Some(hasher) = active.hasher.as_mut() {
            hasher.update(data);
        }
        active.next_offset += data.len() as u64;
        Ok(())
    }

    pub async fn finish_file(
        &mut self,
        file_id: u32,
        declared_size: u64,
        declared_hash: &str,
    ) -> Result<FinalizedFile, TransferError> {
        let active = self
            .active
            .take()
            .ok_or_else(|| TransferError::State("no active file".into()))?;
        if active.file_id != file_id {
            return Err(TransferError::State(
                "file completion for unexpected file".into(),
            ));
        }
        let expected_size = self.file(file_id)?.size;
        if declared_size != expected_size || active.next_offset != expected_size {
            return Err(TransferError::State(
                "received size does not match metadata".into(),
            ));
        }
        let mut file = active.file;
        file.flush().await?;
        file.sync_data().await?;
        drop(file);

        let path = part_path(&self.state_dir, file_id);
        // Fresh transfers hash every received chunk incrementally, so no
        // second full read of 1.7 GB is needed. Resumed files have no hasher
        // and are re-read to cover the prefix from the earlier session.
        let actual = match active.hasher {
            Some(hasher) => hasher.finalize(),
            None => hash_file(path.clone()).await?,
        };
        if actual.to_hex().as_str() != declared_hash {
            return Err(TransferError::VerificationFailed {
                path: self.file(file_id)?.rel_path.clone(),
            });
        }

        let rel = verify_wire_path(&self.file(file_id)?.rel_path)
            .map_err(|e| TransferError::State(e.to_string()))?;
        let reserved = reserve_unique(&self.dest_root, &rel)?;
        if let Err(err) = tokio::fs::rename(&path, &reserved).await {
            let _ = tokio::fs::remove_file(&reserved).await;
            return Err(TransferError::Io(err));
        }
        let final_rel = wire_path_below(&self.dest_root, &reserved)
            .ok_or_else(|| TransferError::State("final path escaped destination".into()))?;

        {
            let entry = self
                .manifest
                .files
                .iter_mut()
                .find(|f| f.file_id == file_id)
                .ok_or_else(|| TransferError::State(format!("unknown file id {file_id}")))?;
            entry.completed = true;
            entry.final_path = Some(final_rel.clone());
            entry.hash = Some(actual.to_hex().to_string());
        }
        save_manifest(&self.state_dir, &self.manifest).await?;
        self.session_completed.insert(file_id);
        info!(
            transfer = %self.transfer_id,
            file = %final_rel,
            bytes = declared_size,
            "file received and verified"
        );

        Ok(FinalizedFile {
            destination: reserved,
            rel_path: final_rel,
            bytes: declared_size,
        })
    }

    pub fn all_accounted(&self) -> bool {
        self.manifest.files.iter().all(|f| {
            self.initial_completed.contains(&f.file_id)
                || self.session_completed.contains(&f.file_id)
        })
    }

    pub fn cleanup_after_success(&self) {
        let state_dir = self.state_dir.clone();
        tokio::spawn(async move {
            match tokio::fs::remove_dir_all(&state_dir).await {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => warn!(error = %err, "failed to remove transfer state"),
            }
            if let Some(root) = state_dir.parent() {
                let _ = tokio::fs::remove_dir(root).await;
            }
        });
    }
}

fn fresh_manifest(request: &TransferRequest) -> Manifest {
    Manifest {
        version: MANIFEST_VERSION,
        transfer_id: request.transfer_id.clone(),
        files: request
            .files
            .iter()
            .map(|f: &FileMeta| ManifestFile {
                file_id: f.file_id,
                rel_path: f.rel_path.clone(),
                size: f.size,
                modified_ms: f.modified_ms,
                completed: false,
                final_path: None,
                hash: None,
            })
            .collect(),
    }
}

fn manifest_matches(manifest: &Manifest, request: &TransferRequest) -> bool {
    manifest.version == MANIFEST_VERSION
        && manifest.transfer_id == request.transfer_id
        && manifest.files.len() == request.files.len()
        && manifest
            .files
            .iter()
            .zip(request.files.iter())
            .all(|(a, b)| {
                a.file_id == b.file_id
                    && a.rel_path == b.rel_path
                    && a.size == b.size
                    && a.modified_ms == b.modified_ms
            })
}

fn load_manifest(path: &Path) -> Result<Option<Manifest>, TransferError> {
    match fs::read(path) {
        Ok(bytes) => {
            let manifest: Manifest = serde_json::from_slice(&bytes)
                .map_err(|e| TransferError::State(format!("invalid manifest: {e}")))?;
            if manifest.version != MANIFEST_VERSION {
                return Err(TransferError::State("unsupported manifest version".into()));
            }
            Ok(Some(manifest))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(TransferError::Io(err)),
    }
}

async fn save_manifest(state_dir: &Path, manifest: &Manifest) -> Result<(), TransferError> {
    let state_dir = state_dir.to_path_buf();
    let manifest = manifest.clone();
    tokio::task::spawn_blocking(move || -> Result<(), TransferError> {
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| TransferError::State(e.to_string()))?;
        let path = state_dir.join("manifest.json");
        let tmp = state_dir.join("manifest.json.tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &path)?;
        Ok(())
    })
    .await
    .map_err(|e| TransferError::State(format!("manifest task failed: {e}")))?
}

async fn reset_state_dir(state_dir: &Path) -> Result<(), TransferError> {
    match tokio::fs::remove_dir_all(state_dir).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(TransferError::Io(err)),
    }
    tokio::fs::create_dir_all(state_dir).await?;
    Ok(())
}

fn part_path(state_dir: &Path, file_id: u32) -> PathBuf {
    state_dir.join(format!("{file_id}.part"))
}

async fn verify_completed_file(
    dest_root: &Path,
    file: &ManifestFile,
) -> Result<bool, TransferError> {
    let Some(final_rel) = &file.final_path else {
        return Ok(false);
    };
    let Some(stored_hash) = &file.hash else {
        return Ok(false);
    };
    let rel = match verify_wire_path(final_rel) {
        Ok(rel) => rel,
        Err(_) => return Ok(false),
    };
    let path = dest_root.join(rel);
    let meta = match tokio::fs::metadata(&path).await {
        Ok(meta) => meta,
        Err(_) => return Ok(false),
    };
    if !meta.is_file() || meta.len() != file.size {
        return Ok(false);
    }
    let hash = hash_file(path).await?;
    Ok(hash.to_hex().as_str() == stored_hash)
}

async fn hash_file(path: PathBuf) -> Result<Hash, TransferError> {
    tokio::task::spawn_blocking(move || -> Result<Hash, TransferError> {
        let mut file = fs::File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; HASH_BUFFER];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher.finalize())
    })
    .await
    .map_err(|e| TransferError::State(format!("hash task failed: {e}")))?
}

async fn hash_prefix(path: PathBuf, len: u64) -> Result<Hash, TransferError> {
    tokio::task::spawn_blocking(move || -> Result<Hash, TransferError> {
        let mut file = fs::File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; HASH_BUFFER];
        let mut remaining = len;
        while remaining > 0 {
            let want = remaining.min(buffer.len() as u64) as usize;
            let read = file.read(&mut buffer[..want])?;
            if read == 0 {
                return Err(TransferError::State(
                    "partial file is shorter than expected".into(),
                ));
            }
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        Ok(hasher.finalize())
    })
    .await
    .map_err(|e| TransferError::State(format!("hash task failed: {e}")))?
}

/// Removes abandoned partial transfers older than `max_age`.
pub fn cleanup_stale_states(dest_root: &Path, max_age: Duration) -> usize {
    let root = dest_root.join(STATE_DIR_NAME);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).ok();
        let stale = modified
            .and_then(|time| now.duration_since(time).ok())
            .map(|age| age > max_age)
            .unwrap_or(false);
        if stale && fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    if removed > 0 {
        let _ = fs::remove_dir(&root);
    }
    removed
}
