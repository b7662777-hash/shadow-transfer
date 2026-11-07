//! Shadow Transfer wire protocol: bounded framing plus versioned control messages.

pub mod frame;
pub mod message;

pub use frame::{
    read_frame, write_frame, Frame, FrameError, FrameKind, HEADER_LEN, MAGIC, MAX_CONTROL_FRAME,
    MAX_DATA_FRAME, WIRE_VERSION,
};
pub use message::{
    validate_device_name, validate_fingerprint, validate_hash, validate_nonce, validate_rel_path,
    validate_transfer_id, Cancel, ErrorMessage, FileAck, FileChunk, FileComplete, FileMeta,
    FileStart, Hello, HelloAck, Message, PairAccept, PairReject, PairRequest, Ping, Pong,
    ResumeEntry, SessionReady, TransferAccept, TransferAck, TransferComplete, TransferReject,
    TransferRequest, ValidationError, MAX_FILES_PER_TRANSFER, MAX_REL_PATH_BYTES, PROTOCOL_VERSION,
};

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("malformed JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("control message envelope must contain exactly `type` and `payload`")]
    MalformedEnvelope,
    #[error("expected a {expected} frame, received {actual}")]
    UnexpectedFrame {
        expected: &'static str,
        actual: &'static str,
    },
}

pub fn decode_control(payload: &[u8]) -> Result<Message, ProtocolError> {
    let value: serde_json::Value = serde_json::from_slice(payload)?;
    let object = value.as_object().ok_or(ProtocolError::MalformedEnvelope)?;
    if object.len() != 2 || !object.contains_key("type") || !object.contains_key("payload") {
        return Err(ProtocolError::MalformedEnvelope);
    }
    let message: Message = serde_json::from_value(value)?;
    message.validate()?;
    Ok(message)
}

pub fn encode_control(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    message.validate()?;
    Ok(serde_json::to_vec(message)?)
}

pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Message, ProtocolError> {
    let frame = read_frame(reader).await?;
    if frame.kind != FrameKind::Control {
        return Err(ProtocolError::UnexpectedFrame {
            expected: "control",
            actual: "data",
        });
    }
    decode_control(&frame.payload)
}

pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Message,
) -> Result<(), ProtocolError> {
    let payload = encode_control(message)?;
    write_frame(writer, FrameKind::Control, &payload).await?;
    Ok(())
}

pub async fn read_data<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<bytes::Bytes, ProtocolError> {
    let frame = read_frame(reader).await?;
    if frame.kind != FrameKind::Data {
        return Err(ProtocolError::UnexpectedFrame {
            expected: "data",
            actual: "control",
        });
    }
    Ok(frame.payload)
}

pub async fn write_data<W: AsyncWrite + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), ProtocolError> {
    write_frame(writer, FrameKind::Data, data).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn messages_roundtrip_over_a_stream() {
        let (mut a, mut b) = duplex(8192);
        let message = Message::Ping(Ping {
            nonce: "00112233445566778899aabbccddeeff".into(),
        });
        write_message(&mut a, &message).await.unwrap();
        let decoded = read_message(&mut b).await.unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn envelope_must_be_exact() {
        let extra = br#"{"type":"PAIR_REJECT","payload":{"reason":"no"},"extra":1}"#;
        assert!(matches!(
            decode_control(extra),
            Err(ProtocolError::MalformedEnvelope)
        ));
        let missing = br#"{"type":"PAIR_REJECT"}"#;
        assert!(matches!(
            decode_control(missing),
            Err(ProtocolError::MalformedEnvelope)
        ));
    }

    #[test]
    fn every_variant_serializes_with_payload() {
        let variants = vec![
            Message::PairAccept(PairAccept {
                fingerprint: "00".repeat(32),
            }),
            Message::PairReject(PairReject {
                reason: "no".into(),
            }),
            Message::SessionReady(SessionReady {
                session_id: "00".repeat(16),
            }),
            Message::TransferAck(TransferAck {
                transfer_id: "00".repeat(16),
            }),
            Message::Cancel(Cancel {
                reason: "user".into(),
            }),
            Message::Pong(Pong {
                nonce: "00".repeat(16),
            }),
        ];
        for message in variants {
            let encoded = encode_control(&message).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            assert!(value.get("type").is_some());
            assert!(value.get("payload").is_some());
            assert_eq!(decode_control(&encoded).unwrap(), message);
        }
    }

    #[tokio::test]
    async fn data_frames_are_readable() {
        let (mut a, mut b) = duplex(8192);
        let payload = vec![7u8; 4096];
        write_data(&mut a, &payload).await.unwrap();
        assert_eq!(&read_data(&mut b).await.unwrap()[..], &payload[..]);
    }
}
