use std::time::Duration;

use shadow_protocol::{read_data, read_message, write_data, write_message, Message};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::error::TransferError;

/// A high-latency Wi-Fi link can leave a single TCP stream far below the PHY
/// rate unless both peers expose a large window. The kernel may clamp this to
/// its own maximum.
const TCP_BUFFER_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn tune_tcp(socket: &TcpStream) {
    let sock = socket2::SockRef::from(socket);
    if let Err(err) = sock.set_recv_buffer_size(TCP_BUFFER_BYTES) {
        tracing::debug!(error = %err, "could not enlarge TCP receive buffer");
    }
    if let Err(err) = sock.set_send_buffer_size(TCP_BUFFER_BYTES) {
        tracing::debug!(error = %err, "could not enlarge TCP send buffer");
    }
}

pub(crate) fn random_nonce() -> Result<([u8; 16], String), TransferError> {
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|e| TransferError::Random(e.to_string()))?;
    Ok((nonce, hex::encode(nonce)))
}

pub(crate) fn parse_nonce(value: &str) -> Result<[u8; 16], TransferError> {
    let bytes = hex::decode(value)
        .map_err(|_| TransferError::ProtocolViolation("invalid nonce encoding".into()))?;
    bytes
        .try_into()
        .map_err(|_| TransferError::ProtocolViolation("invalid nonce length".into()))
}

pub(crate) fn new_transfer_id() -> Result<String, TransferError> {
    let (_, id) = random_nonce()?;
    Ok(id)
}

pub(crate) fn peer_fingerprint(
    certificates: Option<&[rustls::pki_types::CertificateDer<'static>]>,
) -> Result<[u8; 32], TransferError> {
    let cert = certificates
        .and_then(|certs| certs.first())
        .ok_or_else(|| TransferError::ProtocolViolation("peer presented no certificate".into()))?;
    Ok(shadow_crypto::fingerprint(cert.as_ref()))
}

pub(crate) async fn recv_message<S: AsyncRead + Unpin>(
    stream: &mut S,
    duration: Duration,
    label: &'static str,
) -> Result<Message, TransferError> {
    match timeout(duration, read_message(stream)).await {
        Err(_) => Err(TransferError::Timeout(label)),
        Ok(Ok(message)) => Ok(message),
        Ok(Err(err)) => Err(err.into()),
    }
}

pub(crate) async fn send_message<S: AsyncWrite + Unpin>(
    stream: &mut S,
    message: &Message,
    duration: Duration,
    label: &'static str,
) -> Result<(), TransferError> {
    match timeout(duration, write_message(stream, message)).await {
        Err(_) => Err(TransferError::Timeout(label)),
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
    }
}

pub(crate) async fn recv_data<S: AsyncRead + Unpin>(
    stream: &mut S,
    duration: Duration,
    label: &'static str,
) -> Result<bytes::Bytes, TransferError> {
    match timeout(duration, read_data(stream)).await {
        Err(_) => Err(TransferError::Timeout(label)),
        Ok(Ok(data)) => Ok(data),
        Ok(Err(err)) => Err(err.into()),
    }
}

pub(crate) async fn send_data<S: AsyncWrite + Unpin>(
    stream: &mut S,
    data: &[u8],
    duration: Duration,
    label: &'static str,
) -> Result<(), TransferError> {
    match timeout(duration, write_data(stream, data)).await {
        Err(_) => Err(TransferError::Timeout(label)),
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.into()),
    }
}

pub(crate) async fn send_cancel_best_effort<S: AsyncWrite + Unpin>(stream: &mut S, reason: &str) {
    let message = Message::Cancel(shadow_protocol::Cancel {
        reason: reason.to_string(),
    });
    let _ = timeout(Duration::from_secs(2), write_message(stream, &message)).await;
}
