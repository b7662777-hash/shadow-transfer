//! Versioned application messages carried inside control frames as JSON.
//!
//! Every payload structure denies unknown fields and every length is bounded.
//! Binary file bytes never appear here; they travel in data frames directly
//! after the corresponding `FILE_CHUNK` control message.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::frame::MAX_DATA_FRAME;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_DEVICE_NAME_CHARS: usize = 64;
pub const MAX_FILES_PER_TRANSFER: usize = 100_000;
pub const MAX_REL_PATH_BYTES: usize = 4096;
pub const MAX_REASON_CHARS: usize = 512;
pub const MAX_FILE_CAP: u64 = 1 << 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol: u16,
    pub device_name: String,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloAck {
    pub protocol: u16,
    pub device_name: String,
    pub fingerprint: String,
    pub nonce: String,
    pub client_trusted: bool,
    pub pairing_available: bool,
    pub pairing_expires_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairRequest {
    pub device_name: String,
    pub fingerprint: String,
    pub proof: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairAccept {
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairReject {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionReady {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileMeta {
    pub file_id: u32,
    pub rel_path: String,
    pub size: u64,
    pub modified_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRequest {
    pub transfer_id: String,
    pub sender_name: String,
    pub files: Vec<FileMeta>,
    pub total_bytes: u64,
}

impl TransferRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_transfer(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeEntry {
    pub file_id: u32,
    pub offset: u64,
    pub prefix_hash: String,
    /// True only when this file was fully received and verified in an earlier
    /// session; `offset` then equals the file size.
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferAccept {
    pub entries: Vec<ResumeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferReject {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileStart {
    pub file_id: u32,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileChunk {
    pub file_id: u32,
    pub offset: u64,
    pub len: u32,
    /// BLAKE3 of the payload in the immediately following data frame.
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileComplete {
    pub file_id: u32,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileAck {
    pub file_id: u32,
    pub verified: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferComplete {
    pub transfer_id: String,
    pub files_verified: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferAck {
    pub transfer_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorMessage {
    pub code: u16,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ping {
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pong {
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Message {
    Hello(Hello),
    HelloAck(HelloAck),
    PairRequest(PairRequest),
    PairAccept(PairAccept),
    PairReject(PairReject),
    SessionReady(SessionReady),
    TransferRequest(TransferRequest),
    TransferAccept(TransferAccept),
    TransferReject(TransferReject),
    FileStart(FileStart),
    FileChunk(FileChunk),
    FileComplete(FileComplete),
    FileAck(FileAck),
    TransferComplete(TransferComplete),
    TransferAck(TransferAck),
    Cancel(Cancel),
    Error(ErrorMessage),
    Ping(Ping),
    Pong(Pong),
}

impl Message {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hello(_) => "HELLO",
            Self::HelloAck(_) => "HELLO_ACK",
            Self::PairRequest(_) => "PAIR_REQUEST",
            Self::PairAccept(_) => "PAIR_ACCEPT",
            Self::PairReject(_) => "PAIR_REJECT",
            Self::SessionReady(_) => "SESSION_READY",
            Self::TransferRequest(_) => "TRANSFER_REQUEST",
            Self::TransferAccept(_) => "TRANSFER_ACCEPT",
            Self::TransferReject(_) => "TRANSFER_REJECT",
            Self::FileStart(_) => "FILE_START",
            Self::FileChunk(_) => "FILE_CHUNK",
            Self::FileComplete(_) => "FILE_COMPLETE",
            Self::FileAck(_) => "FILE_ACK",
            Self::TransferComplete(_) => "TRANSFER_COMPLETE",
            Self::TransferAck(_) => "TRANSFER_ACK",
            Self::Cancel(_) => "CANCEL",
            Self::Error(_) => "ERROR",
            Self::Ping(_) => "PING",
            Self::Pong(_) => "PONG",
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Hello(m) => {
                check_protocol(m.protocol)?;
                validate_device_name(&m.device_name)?;
                validate_nonce(&m.nonce)?;
            }
            Self::HelloAck(m) => {
                check_protocol(m.protocol)?;
                validate_device_name(&m.device_name)?;
                validate_fingerprint(&m.fingerprint)?;
                validate_nonce(&m.nonce)?;
            }
            Self::PairRequest(m) => {
                validate_device_name(&m.device_name)?;
                validate_fingerprint(&m.fingerprint)?;
                validate_hash(&m.proof)?;
            }
            Self::PairAccept(m) => validate_fingerprint(&m.fingerprint)?,
            Self::PairReject(m) => {
                validate_reason(&m.reason)?;
            }
            Self::TransferReject(m) => {
                validate_reason(&m.reason)?;
            }
            Self::SessionReady(m) => validate_transfer_id(&m.session_id)?,
            Self::TransferRequest(m) => {
                validate_transfer_id(&m.transfer_id)?;
                validate_device_name(&m.sender_name)?;
                validate_transfer(m)?;
            }
            Self::TransferAccept(m) => {
                if m.entries.len() > MAX_FILES_PER_TRANSFER {
                    return Err(ValidationError::TooManyFiles(m.entries.len()));
                }
                for entry in &m.entries {
                    validate_hash(&entry.prefix_hash)?;
                }
            }
            Self::FileStart(m) => {
                let _ = m.file_id;
                let _ = m.offset;
            }
            Self::FileChunk(m) => {
                let _ = m.file_id;
                let _ = m.offset;
                if m.len == 0 || m.len as usize > MAX_DATA_FRAME {
                    return Err(ValidationError::BadChunkLength(m.len as usize));
                }
                validate_hash(&m.hash)?;
            }
            Self::FileComplete(m) => {
                let _ = m.file_id;
                let _ = m.size;
                validate_hash(&m.hash)?;
            }
            Self::FileAck(m) => {
                let _ = m.file_id;
                validate_reason(&m.reason)?;
            }
            Self::TransferComplete(m) => {
                validate_transfer_id(&m.transfer_id)?;
                let _ = m.files_verified;
            }
            Self::TransferAck(m) => validate_transfer_id(&m.transfer_id)?,
            Self::Cancel(m) => validate_reason(&m.reason)?,
            Self::Error(m) => {
                let _ = m.code;
                if m.message.len() > 2048 {
                    return Err(ValidationError::FieldTooLong {
                        field: "message",
                        len: m.message.len(),
                        max: 2048,
                    });
                }
            }
            Self::Ping(m) => validate_nonce(&m.nonce)?,
            Self::Pong(m) => validate_nonce(&m.nonce)?,
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("unsupported protocol version {0}")]
    BadProtocolVersion(u16),
    #[error("invalid device name")]
    BadDeviceName,
    #[error("field `{field}` has invalid length {len} (max {max})")]
    FieldTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    #[error("field `{field}` must be hex encoded")]
    BadHex { field: &'static str },
    #[error("field `{field}` has wrong byte length {len} (expected {expected})")]
    BadHexLength {
        field: &'static str,
        len: usize,
        expected: usize,
    },
    #[error("unsafe relative path")]
    UnsafePath,
    #[error("transfer contains {0} files, over the limit")]
    TooManyFiles(usize),
    #[error("transfer declares total {declared} but files sum to {actual}")]
    TotalMismatch { declared: u64, actual: u64 },
    #[error("file size {0} exceeds supported maximum")]
    FileTooLarge(u64),
    #[error("invalid chunk length {0}")]
    BadChunkLength(usize),
    #[error("reason text too long")]
    ReasonTooLong,
}

fn check_protocol(version: u16) -> Result<(), ValidationError> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ValidationError::BadProtocolVersion(version))
    }
}

pub fn validate_device_name(name: &str) -> Result<(), ValidationError> {
    if name.trim().is_empty()
        || name.chars().count() > MAX_DEVICE_NAME_CHARS
        || name.chars().any(|c| c.is_control())
    {
        return Err(ValidationError::BadDeviceName);
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<(), ValidationError> {
    if reason.chars().count() > MAX_REASON_CHARS || reason.chars().any(|c| c == '\0') {
        return Err(ValidationError::ReasonTooLong);
    }
    Ok(())
}

fn check_hex(
    field: &'static str,
    value: &str,
    expected_bytes: usize,
) -> Result<(), ValidationError> {
    if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ValidationError::BadHex { field });
    }
    if value.len() != expected_bytes * 2 {
        return Err(ValidationError::BadHexLength {
            field,
            len: value.len(),
            expected: expected_bytes,
        });
    }
    Ok(())
}

pub fn validate_nonce(value: &str) -> Result<(), ValidationError> {
    check_hex("nonce", value, 16)
}

pub fn validate_fingerprint(value: &str) -> Result<(), ValidationError> {
    check_hex("fingerprint", value, 32)
}

pub fn validate_hash(value: &str) -> Result<(), ValidationError> {
    check_hex("hash", value, 32)
}

pub fn validate_transfer_id(value: &str) -> Result<(), ValidationError> {
    check_hex("transfer_id", value, 16)
}

pub fn validate_rel_path(path: &str) -> Result<(), ValidationError> {
    if path.is_empty() || path.len() > MAX_REL_PATH_BYTES {
        return Err(ValidationError::UnsafePath);
    }
    if path.contains('\0') || path.contains('\\') || path.starts_with('/') {
        return Err(ValidationError::UnsafePath);
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(ValidationError::UnsafePath);
        }
        if component.chars().any(|c| c.is_control()) {
            return Err(ValidationError::UnsafePath);
        }
    }
    Ok(())
}

pub fn validate_transfer(request: &TransferRequest) -> Result<(), ValidationError> {
    if request.files.is_empty() || request.files.len() > MAX_FILES_PER_TRANSFER {
        return Err(ValidationError::TooManyFiles(request.files.len()));
    }
    let mut total = 0u64;
    let mut seen_paths = std::collections::HashSet::with_capacity(request.files.len());
    for file in &request.files {
        validate_rel_path(&file.rel_path)?;
        if file.size > MAX_FILE_CAP {
            return Err(ValidationError::FileTooLarge(file.size));
        }
        if !seen_paths.insert(file.rel_path.as_str()) {
            return Err(ValidationError::UnsafePath);
        }
        total = total
            .checked_add(file.size)
            .ok_or(ValidationError::TotalMismatch {
                declared: request.total_bytes,
                actual: u64::MAX,
            })?;
    }
    if total != request.total_bytes {
        return Err(ValidationError::TotalMismatch {
            declared: request.total_bytes,
            actual: total,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: u32, path: &str, size: u64) -> FileMeta {
        FileMeta {
            file_id: id,
            rel_path: path.to_string(),
            size,
            modified_ms: 0,
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        let json = r#"{"type":"HELLO","payload":{"protocol":1,"device_name":"pc","nonce":"00000000000000000000000000000000","evil":1}}"#;
        assert!(serde_json::from_str::<Message>(json).is_err());
    }

    #[test]
    fn accepts_valid_hello() {
        let message = Message::Hello(Hello {
            protocol: PROTOCOL_VERSION,
            device_name: "Workstation".into(),
            nonce: "00112233445566778899aabbccddeeff".into(),
        });
        message.validate().unwrap();
        let json = serde_json::to_string(&message).unwrap();
        let decoded: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn rejects_path_traversal_in_transfer() {
        let request = TransferRequest {
            transfer_id: "00112233445566778899aabbccddeeff".into(),
            sender_name: "pc".into(),
            files: vec![meta(1, "../escape.txt", 4)],
            total_bytes: 4,
        };
        assert_eq!(request.validate(), Err(ValidationError::UnsafePath));
    }

    #[test]
    fn rejects_duplicate_paths() {
        let request = TransferRequest {
            transfer_id: "00112233445566778899aabbccddeeff".into(),
            sender_name: "pc".into(),
            files: vec![meta(1, "a.txt", 1), meta(2, "a.txt", 2)],
            total_bytes: 3,
        };
        assert_eq!(request.validate(), Err(ValidationError::UnsafePath));
    }

    #[test]
    fn rejects_total_mismatch() {
        let request = TransferRequest {
            transfer_id: "00112233445566778899aabbccddeeff".into(),
            sender_name: "pc".into(),
            files: vec![meta(1, "a.txt", 100)],
            total_bytes: 99,
        };
        assert!(matches!(
            request.validate(),
            Err(ValidationError::TotalMismatch { .. })
        ));
    }

    #[test]
    fn rejects_bad_hash_and_chunk_size() {
        let message = Message::FileChunk(FileChunk {
            file_id: 1,
            offset: 0,
            len: 0,
            hash: "00".repeat(32),
        });
        assert!(matches!(
            message.validate(),
            Err(ValidationError::BadChunkLength(0))
        ));

        let message = Message::FileChunk(FileChunk {
            file_id: 1,
            offset: 0,
            len: 1024,
            hash: "not-hex".into(),
        });
        assert!(matches!(
            message.validate(),
            Err(ValidationError::BadHex { .. })
        ));
    }

    #[test]
    fn rejects_protocol_downgrade() {
        let message = Message::Hello(Hello {
            protocol: 0,
            device_name: "pc".into(),
            nonce: "00112233445566778899aabbccddeeff".into(),
        });
        assert_eq!(
            message.validate(),
            Err(ValidationError::BadProtocolVersion(0))
        );
    }

    #[test]
    fn rejects_absurd_file_counts() {
        let request = TransferRequest {
            transfer_id: "00112233445566778899aabbccddeeff".into(),
            sender_name: "pc".into(),
            files: vec![],
            total_bytes: 0,
        };
        assert!(matches!(
            request.validate(),
            Err(ValidationError::TooManyFiles(0))
        ));
    }
}
