use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub device_name: String,
    pub download_dir: PathBuf,
    pub receive_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: computer_name(),
            download_dir: PathBuf::new(),
            receive_port: 47831,
        }
    }
}

impl Settings {
    pub fn with_default_download(mut self, dir: PathBuf) -> Self {
        if self.download_dir.as_os_str().is_empty() {
            self.download_dir = dir;
        }
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        let name = self.device_name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(|c| c.is_control()) {
            return Err("Device name must be 1–64 printable characters".into());
        }
        if self.receive_port < 1024 {
            return Err("Receive port must be 1024 or higher".into());
        }
        if self.download_dir.as_os_str().is_empty() {
            return Err("Choose a download folder".into());
        }
        Ok(())
    }

    pub fn load(path: &Path, fallback_download: PathBuf) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default().with_default_download(fallback_download));
        }
        let bytes = fs::read(path).map_err(|e| format!("cannot read settings: {e}"))?;
        let settings: Settings =
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid settings: {e}"))?;
        Ok(settings.with_default_download(fallback_download))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        write_json_atomic(path, self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub at: u64,
    pub direction: String,
    pub peer: String,
    pub files: usize,
    pub bytes: u64,
    pub status: String,
    pub detail: String,
    pub transfer_id: String,
}

pub fn data_dir() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir()
        .ok_or_else(|| "Windows local application data directory is unavailable".to_string())?;
    let dir = base.join("ShadowTransfer");
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| {
            !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(|c| c.is_control())
        })
        .unwrap_or_else(|| "Windows PC".to_string())
}

pub fn load_history(path: &Path) -> Result<Vec<HistoryEntry>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path).map_err(|e| format!("cannot read history: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid history file: {e}"))
}

pub fn save_history(path: &Path, history: &[HistoryEntry]) -> Result<(), String> {
    write_json_atomic(path, history)
}

pub fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| format!("cannot serialize {}: {e}", path.display()))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file =
            fs::File::create(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        file.write_all(&bytes)
            .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("cannot flush {}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("cannot finalize {}: {e}", path.display()))?;
    Ok(())
}

pub fn relative_time(timestamp: u64) -> String {
    let now = now_unix();
    if now <= timestamp {
        return "just now".to_string();
    }
    let seconds = now - timestamp;
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}
