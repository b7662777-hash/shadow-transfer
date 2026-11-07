//! Length-prefixed framing shared by both platforms.
//!
//! Wire layout (all integers little-endian):
//!
//! ```text
//! 0..4   magic "STX1"
//! 4      frame format version (1)
//! 5      kind: 0 = control (UTF-8 JSON), 1 = data (raw file bytes)
//! 6..8   reserved flags, must be zero
//! 8..12  payload length (u32)
//! 12..   payload
//! ```
//!
//! Frames are bounded before any allocation happens, so a hostile peer cannot
//! make us reserve arbitrarily large buffers.

use bytes::Bytes;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAGIC: [u8; 4] = *b"STX1";
pub const WIRE_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 12;
pub const MAX_CONTROL_FRAME: usize = 256 * 1024;
pub const MAX_DATA_FRAME: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Control,
    Data,
}

impl FrameKind {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Control => 0,
            Self::Data => 1,
        }
    }

    pub const fn max_payload(self) -> usize {
        match self {
            Self::Control => MAX_CONTROL_FRAME,
            Self::Data => MAX_DATA_FRAME,
        }
    }

    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Control),
            1 => Some(Self::Data),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub kind: FrameKind,
    pub payload: Bytes,
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid frame magic")]
    BadMagic,
    #[error("unsupported frame version {0}")]
    UnsupportedVersion(u8),
    #[error("unknown frame kind {0}")]
    UnknownKind(u8),
    #[error("reserved frame flags are set: {0:#06x}")]
    ReservedFlags(u16),
    #[error("frame of {len} bytes exceeds limit of {max} bytes for {kind:?}")]
    Oversized {
        kind: FrameKind,
        len: usize,
        max: usize,
    },
    #[error("empty control frame")]
    EmptyControl,
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Frame, FrameError> {
    let mut header = [0u8; HEADER_LEN];
    reader.read_exact(&mut header).await?;

    if header[0..4] != MAGIC {
        return Err(FrameError::BadMagic);
    }
    if header[4] != WIRE_VERSION {
        return Err(FrameError::UnsupportedVersion(header[4]));
    }
    let kind = FrameKind::from_u8(header[5]).ok_or(FrameError::UnknownKind(header[5]))?;
    let flags = u16::from_le_bytes([header[6], header[7]]);
    if flags != 0 {
        return Err(FrameError::ReservedFlags(flags));
    }
    let len = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    let max = kind.max_payload();
    if len > max {
        return Err(FrameError::Oversized { kind, len, max });
    }
    if kind == FrameKind::Control && len == 0 {
        return Err(FrameError::EmptyControl);
    }

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Frame {
        kind,
        payload: Bytes::from(payload),
    })
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    kind: FrameKind,
    payload: &[u8],
) -> std::io::Result<()> {
    let max = kind.max_payload();
    if payload.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("frame payload {} exceeds limit {max}", payload.len()),
        ));
    }
    let len = u32::try_from(payload.len()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame payload too large")
    })?;

    let mut header = [0u8; HEADER_LEN];
    header[0..4].copy_from_slice(&MAGIC);
    header[4] = WIRE_VERSION;
    header[5] = kind.as_u8();
    header[6..8].copy_from_slice(&0u16.to_le_bytes());
    header[8..12].copy_from_slice(&len.to_le_bytes());

    writer.write_all(&header).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode(kind: FrameKind, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut header = [0u8; HEADER_LEN];
        header[0..4].copy_from_slice(&MAGIC);
        header[4] = WIRE_VERSION;
        header[5] = kind.as_u8();
        header[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(payload);
        out
    }

    #[tokio::test]
    async fn roundtrips_frames() {
        let raw = encode(FrameKind::Control, b"{\"a\":1}");
        let mut cursor = Cursor::new(raw);
        let frame = read_frame(&mut cursor).await.unwrap();
        assert_eq!(frame.kind, FrameKind::Control);
        assert_eq!(&frame.payload[..], b"{\"a\":1}");
    }

    #[tokio::test]
    async fn rejects_oversized_frames_before_allocating() {
        let mut raw = encode(FrameKind::Control, b"");
        raw[8..12].copy_from_slice(&(u32::MAX).to_le_bytes());
        let mut cursor = Cursor::new(raw);
        let err = read_frame(&mut cursor).await.unwrap_err();
        assert!(matches!(err, FrameError::Oversized { .. }));
    }

    #[tokio::test]
    async fn rejects_bad_magic_and_flags() {
        let mut raw = encode(FrameKind::Control, b"{}");
        raw[0] = b'X';
        assert!(matches!(
            read_frame(&mut Cursor::new(raw)).await.unwrap_err(),
            FrameError::BadMagic
        ));

        let mut raw = encode(FrameKind::Control, b"{}");
        raw[6] = 1;
        assert!(matches!(
            read_frame(&mut Cursor::new(raw)).await.unwrap_err(),
            FrameError::ReservedFlags(1)
        ));
    }

    #[tokio::test]
    async fn rejects_empty_control_and_unknown_kind() {
        let raw = encode(FrameKind::Control, b"");
        assert!(matches!(
            read_frame(&mut Cursor::new(raw)).await.unwrap_err(),
            FrameError::EmptyControl
        ));

        let mut raw = encode(FrameKind::Control, b"{}");
        raw[5] = 9;
        assert!(matches!(
            read_frame(&mut Cursor::new(raw)).await.unwrap_err(),
            FrameError::UnknownKind(9)
        ));
    }
}
