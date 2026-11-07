use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use blake3::Hasher;
use shadow_crypto::{pairing_proof, unix_now, Identity, TrustStore, ALPN_PROTOCOL};
use shadow_protocol::{
    Cancel, FileChunk, FileComplete, FileStart, Hello, Message, PairRequest, TransferAccept,
    TransferComplete, TransferRequest, PROTOCOL_VERSION,
};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::sleep;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::error::TransferError;
use crate::path::verify_wire_path;
use crate::progress::{Direction, ProgressEvent, ProgressTracker, TransferPhase};
use crate::session::{
    new_transfer_id, parse_nonce, peer_fingerprint, random_nonce, recv_message,
    send_cancel_best_effort, send_data, send_message,
};
use crate::source::SourceFile;

const MIN_ADAPTIVE_CHUNK: usize = 256 * 1024;
const MAX_ADAPTIVE_CHUNK: usize = 4 * 1024 * 1024;
const MIN_CHUNK: usize = 16 * 1024;
const HASH_BUFFER: usize = 256 * 1024;

#[derive(Debug, Clone, Copy)]
pub enum ChunkPolicy {
    Adaptive,
    Fixed(usize),
}

#[derive(Debug, Clone)]
pub struct SenderConfig {
    pub peer: SocketAddr,
    pub identity: Arc<Identity>,
    pub trust: Arc<TrustStore>,
    pub device_name: String,
    pub pairing_code: Option<String>,
    pub chunk_policy: ChunkPolicy,
    pub connect_timeout: Duration,
    pub handshake_timeout: Duration,
    pub approval_timeout: Duration,
    pub io_timeout: Duration,
    pub ack_timeout: Duration,
    pub max_attempts: u32,
}

impl SenderConfig {
    pub fn new(
        peer: SocketAddr,
        identity: Arc<Identity>,
        trust: Arc<TrustStore>,
        device_name: impl Into<String>,
    ) -> Self {
        Self {
            peer,
            identity,
            trust,
            device_name: device_name.into(),
            pairing_code: None,
            chunk_policy: ChunkPolicy::Adaptive,
            connect_timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_secs(15),
            approval_timeout: Duration::from_secs(180),
            io_timeout: Duration::from_secs(60),
            ack_timeout: Duration::from_secs(600),
            max_attempts: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PairSummary {
    pub peer_name: String,
    pub peer_fingerprint: String,
}

/// Establishes (or verifies) trust without sending any files.
///
/// The connection is closed with a `CANCEL` before a transfer request, which
/// the receiver treats as a clean no-op. This lets the UI separate "connect and
/// pair" from "choose files and send".
pub async fn pair_with(cfg: &SenderConfig) -> Result<PairSummary, TransferError> {
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let drain = tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
    let mut tracker = ProgressTracker::new(progress_tx, "pairing", Direction::Send, 0, 0);
    let session = handshake(cfg, &mut tracker).await;
    drop(tracker);
    let _ = drain.await;
    let mut session = session?;
    let _ = send_message(
        &mut session.stream,
        &Message::Cancel(Cancel {
            reason: "pairing complete".into(),
        }),
        cfg.io_timeout,
        "close pairing session",
    )
    .await;
    Ok(PairSummary {
        peer_name: session.peer_name,
        peer_fingerprint: session.peer_fingerprint,
    })
}

#[derive(Debug, Clone)]
pub struct FileOutcome {
    pub file_id: u32,
    pub rel_path: String,
    pub size: u64,
    pub resumed_from: u64,
    pub skipped: bool,
}

#[derive(Debug, Clone)]
pub struct SendSummary {
    pub transfer_id: String,
    pub peer_name: String,
    pub peer_fingerprint: String,
    pub files: Vec<FileOutcome>,
    pub bytes_sent: u64,
    pub bytes_reused: u64,
    pub duration: Duration,
    pub avg_bps: f64,
}

pub async fn send_sources(
    cfg: &SenderConfig,
    transfer_id: Option<String>,
    sources: Vec<SourceFile>,
    progress: UnboundedSender<ProgressEvent>,
    cancel: CancellationToken,
) -> Result<SendSummary, TransferError> {
    if sources.is_empty() {
        return Err(TransferError::EmptySelection);
    }
    let mut total_bytes = 0u64;
    let mut metas = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        verify_wire_path(&source.rel_path).map_err(|e| TransferError::UnsafePath(e.to_string()))?;
        let file_id = index as u32;
        metas.push(source.to_meta(file_id));
        total_bytes = total_bytes
            .checked_add(source.size)
            .ok_or_else(|| TransferError::InvalidSource("selection is too large".into()))?;
    }
    let transfer_id = match transfer_id {
        Some(id) => {
            shadow_protocol::validate_transfer_id(&id)
                .map_err(|e| TransferError::UnsafePath(e.to_string()))?;
            id
        }
        None => new_transfer_id()?,
    };
    let request = TransferRequest {
        transfer_id: transfer_id.clone(),
        sender_name: cfg.device_name.clone(),
        files: metas,
        total_bytes,
    };
    request.validate().map_err(TransferError::Validation)?;

    let mut tracker = ProgressTracker::new(
        progress,
        &transfer_id,
        Direction::Send,
        sources.len(),
        total_bytes,
    );
    let started = Instant::now();
    let attempts = cfg.max_attempts.max(1);

    for attempt in 0..attempts {
        if cancel.is_cancelled() {
            tracker.set_phase(TransferPhase::Cancelled);
            return Err(TransferError::Cancelled);
        }
        if attempt > 0 {
            tracker.restart();
            warn!(attempt, "retrying transfer after connection failure");
            sleep(Duration::from_millis(500)).await;
        }
        match send_once(cfg, &request, &sources, &mut tracker, &cancel).await {
            Ok(mut summary) => {
                summary.duration = started.elapsed();
                summary.avg_bps = if summary.duration.as_secs_f64() > 0.0 {
                    summary.bytes_sent as f64 / summary.duration.as_secs_f64()
                } else {
                    0.0
                };
                tracker.set_phase(TransferPhase::Completed);
                info!(
                    transfer = %transfer_id,
                    peer = %summary.peer_name,
                    bytes = summary.bytes_sent,
                    "transfer complete"
                );
                return Ok(summary);
            }
            Err(err) => {
                if cancel.is_cancelled() {
                    tracker.set_phase(TransferPhase::Cancelled);
                    return Err(TransferError::Cancelled);
                }
                if err.is_retryable() && attempt + 1 < attempts {
                    continue;
                }
                tracker.set_phase(TransferPhase::Failed);
                return Err(err);
            }
        }
    }
    tracker.set_phase(TransferPhase::Failed);
    Err(TransferError::Io(std::io::Error::other(
        "transfer failed after retries",
    )))
}

struct ClientSession {
    stream: tokio_rustls::client::TlsStream<TcpStream>,
    peer_name: String,
    peer_fingerprint: String,
}

async fn send_once(
    cfg: &SenderConfig,
    request: &TransferRequest,
    sources: &[SourceFile],
    tracker: &mut ProgressTracker,
    cancel: &CancellationToken,
) -> Result<SendSummary, TransferError> {
    let mut session = handshake(cfg, tracker).await?;

    send_message(
        &mut session.stream,
        &Message::TransferRequest(request.clone()),
        cfg.io_timeout,
        "send transfer request",
    )
    .await?;
    tracker.set_phase(TransferPhase::Pairing);
    let response = recv_message(
        &mut session.stream,
        cfg.approval_timeout,
        "await transfer approval",
    )
    .await?;
    let accept = match response {
        Message::TransferAccept(accept) => accept,
        Message::TransferReject(reject) => {
            return Err(TransferError::TransferRejected(reject.reason))
        }
        Message::Error(error) => {
            return Err(TransferError::protocol_error(error.code, error.message))
        }
        Message::Cancel(cancel_message) => {
            return Err(TransferError::TransferRejected(format!(
                "cancelled by receiver: {}",
                cancel_message.reason
            )))
        }
        other => {
            return Err(TransferError::ProtocolViolation(format!(
                "expected TRANSFER_ACCEPT, received {}",
                other.name()
            )))
        }
    };

    let entries = validate_entries(request, &accept)?;
    tracker.set_phase(TransferPhase::Transferring);

    let mut planner = ChunkPlanner::new(cfg.chunk_policy);
    let mut buffer = vec![0u8; MAX_ADAPTIVE_CHUNK.max(MIN_CHUNK)];
    let mut outcomes = Vec::with_capacity(request.files.len());
    let mut bytes_sent = 0u64;
    let mut bytes_reused = 0u64;

    for (index, (meta, source)) in request.files.iter().zip(sources.iter()).enumerate() {
        if cancel.is_cancelled() {
            send_cancel_best_effort(&mut session.stream, "cancelled by user").await;
            return Err(TransferError::Cancelled);
        }
        let mut file = match File::open(&source.path).await {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let message = format!("source file disappeared: {}", source.path.display());
                send_cancel_best_effort(&mut session.stream, &message).await;
                return Err(TransferError::SourceChanged(message));
            }
            Err(err) => return Err(TransferError::Io(err)),
        };
        let stat = file.metadata().await?;
        let stat_mtime = stat
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if stat.len() != meta.size || stat_mtime != meta.modified_ms {
            send_cancel_best_effort(&mut session.stream, "source file changed").await;
            return Err(TransferError::SourceChanged(meta.rel_path.clone()));
        }

        let planned = entries.get(&meta.file_id);
        let mut offset = planned
            .map(|entry| entry.offset.min(meta.size))
            .unwrap_or(0);
        let mut hasher = Hasher::new();
        if offset > 0 {
            let prefix = hash_prefix_into(&mut file, offset, &mut hasher).await?;
            let matches = planned
                .map(|entry| {
                    entry
                        .prefix_hash
                        .eq_ignore_ascii_case(prefix.to_hex().as_str())
                })
                .unwrap_or(false);
            if !matches {
                warn!(file = %meta.rel_path, "resume prefix does not match; restarting file");
                offset = 0;
                hasher = Hasher::new();
                file.seek(SeekFrom::Start(0)).await?;
            }
        }

        let completed_on_receiver = planned.map(|entry| entry.completed).unwrap_or(false);
        if completed_on_receiver && offset == meta.size {
            bytes_reused += meta.size;
            tracker.begin_file(index, &meta.rel_path, meta.size, meta.size);
            tracker.finish_file();
            info!(file = %meta.rel_path, "verified file already present on receiver");
            outcomes.push(FileOutcome {
                file_id: meta.file_id,
                rel_path: meta.rel_path.clone(),
                size: meta.size,
                resumed_from: meta.size,
                skipped: true,
            });
            continue;
        }

        if offset > 0 {
            bytes_reused += offset;
        }
        tracker.begin_file(index, &meta.rel_path, meta.size, offset);
        send_message(
            &mut session.stream,
            &Message::FileStart(FileStart {
                file_id: meta.file_id,
                offset,
            }),
            cfg.io_timeout,
            "send file start",
        )
        .await?;

        let mut remaining = meta.size.saturating_sub(offset);
        while remaining > 0 {
            if cancel.is_cancelled() {
                send_cancel_best_effort(&mut session.stream, "cancelled by user").await;
                return Err(TransferError::Cancelled);
            }
            let want = planner.next_chunk().min(remaining as usize);
            let filled = read_full(&mut file, &mut buffer[..want]).await?;
            if filled != want {
                send_cancel_best_effort(&mut session.stream, "source file changed").await;
                return Err(TransferError::SourceChanged(meta.rel_path.clone()));
            }
            let chunk_hash = blake3::hash(&buffer[..want]);
            let chunk_offset = meta.size - remaining;
            send_message(
                &mut session.stream,
                &Message::FileChunk(FileChunk {
                    file_id: meta.file_id,
                    offset: chunk_offset,
                    len: want as u32,
                    hash: chunk_hash.to_hex().to_string(),
                }),
                cfg.io_timeout,
                "send chunk header",
            )
            .await?;
            send_data(
                &mut session.stream,
                &buffer[..want],
                cfg.io_timeout,
                "send chunk",
            )
            .await?;
            hasher.update(&buffer[..want]);
            remaining -= want as u64;
            bytes_sent += want as u64;
            planner.record(want as u64);
            tracker.add_bytes(want as u64);
        }

        let full_hash = hasher.finalize().to_hex().to_string();
        send_message(
            &mut session.stream,
            &Message::FileComplete(FileComplete {
                file_id: meta.file_id,
                size: meta.size,
                hash: full_hash.clone(),
            }),
            cfg.io_timeout,
            "send file completion",
        )
        .await?;

        tracker.set_phase(TransferPhase::Verifying);
        let ack = recv_message(
            &mut session.stream,
            cfg.ack_timeout,
            "await file verification",
        )
        .await?;
        match ack {
            Message::FileAck(ack) if ack.verified && ack.file_id == meta.file_id => {}
            Message::FileAck(ack) => {
                if !ack.reason.is_empty() {
                    warn!(file = %meta.rel_path, reason = %ack.reason, "receiver rejected file");
                }
                return Err(TransferError::VerificationFailed {
                    path: meta.rel_path.clone(),
                });
            }
            Message::Error(error) => {
                return Err(TransferError::protocol_error(error.code, error.message))
            }
            Message::Cancel(cancel_message) => {
                return Err(TransferError::TransferRejected(format!(
                    "cancelled by receiver: {}",
                    cancel_message.reason
                )))
            }
            other => {
                return Err(TransferError::ProtocolViolation(format!(
                    "expected FILE_ACK, received {}",
                    other.name()
                )))
            }
        }
        tracker.set_phase(TransferPhase::Transferring);
        tracker.finish_file();
        outcomes.push(FileOutcome {
            file_id: meta.file_id,
            rel_path: meta.rel_path.clone(),
            size: meta.size,
            resumed_from: offset,
            skipped: false,
        });
    }

    send_message(
        &mut session.stream,
        &Message::TransferComplete(TransferComplete {
            transfer_id: request.transfer_id.clone(),
            files_verified: outcomes.len() as u32,
        }),
        cfg.io_timeout,
        "send transfer completion",
    )
    .await?;
    let ack = recv_message(
        &mut session.stream,
        cfg.ack_timeout,
        "await transfer acknowledgement",
    )
    .await?;
    match ack {
        Message::TransferAck(ack) if ack.transfer_id == request.transfer_id => {}
        Message::Error(error) => {
            return Err(TransferError::protocol_error(error.code, error.message))
        }
        other => {
            return Err(TransferError::ProtocolViolation(format!(
                "expected TRANSFER_ACK, received {}",
                other.name()
            )))
        }
    }

    Ok(SendSummary {
        transfer_id: request.transfer_id.clone(),
        peer_name: session.peer_name,
        peer_fingerprint: session.peer_fingerprint,
        files: outcomes,
        bytes_sent,
        bytes_reused,
        duration: Duration::ZERO,
        avg_bps: 0.0,
    })
}

async fn handshake(
    cfg: &SenderConfig,
    tracker: &mut ProgressTracker,
) -> Result<ClientSession, TransferError> {
    tracker.set_phase(TransferPhase::Connecting);
    let tcp = tokio::time::timeout(cfg.connect_timeout, TcpStream::connect(cfg.peer))
        .await
        .map_err(|_| TransferError::Timeout("TCP connect"))??;
    tcp.set_nodelay(true)?;
    crate::session::tune_tcp(&tcp);

    let connector = TlsConnector::from(shadow_crypto::client_config(&cfg.identity)?);
    let server_name = rustls::pki_types::ServerName::try_from("shadow-transfer.invalid")
        .map_err(|_| TransferError::ProtocolViolation("invalid TLS server name".into()))?;
    let mut stream =
        tokio::time::timeout(cfg.handshake_timeout, connector.connect(server_name, tcp))
            .await
            .map_err(|_| TransferError::Timeout("TLS handshake"))??;

    if stream.get_ref().1.alpn_protocol() != Some(ALPN_PROTOCOL) {
        return Err(TransferError::ProtocolViolation(
            "ALPN negotiation failed".into(),
        ));
    }
    let peer_fp = peer_fingerprint(stream.get_ref().1.peer_certificates())?;
    let peer_fp_hex = hex::encode(peer_fp);

    let (client_nonce, client_nonce_hex) = random_nonce()?;
    send_message(
        &mut stream,
        &Message::Hello(Hello {
            protocol: PROTOCOL_VERSION,
            device_name: cfg.device_name.clone(),
            nonce: client_nonce_hex,
        }),
        cfg.handshake_timeout,
        "send hello",
    )
    .await?;

    let ack = recv_message(&mut stream, cfg.handshake_timeout, "await hello ack").await?;
    let ack = match ack {
        Message::HelloAck(ack) => ack,
        Message::Error(error) => {
            return Err(TransferError::protocol_error(error.code, error.message))
        }
        Message::PairReject(reject) => return Err(TransferError::PairingRejected(reject.reason)),
        other => {
            return Err(TransferError::ProtocolViolation(format!(
                "expected HELLO_ACK, received {}",
                other.name()
            )))
        }
    };
    if !ack.fingerprint.eq_ignore_ascii_case(&peer_fp_hex) {
        return Err(TransferError::ProtocolViolation(
            "receiver fingerprint does not match its certificate".into(),
        ));
    }
    let server_nonce = parse_nonce(&ack.nonce)?;
    let server_pinned = cfg.trust.is_trusted(&peer_fp);

    if !server_pinned && ack.client_trusted {
        // The receiver remembers us, but we have no pin for it (for example
        // after a reinstall). Proceeding would mean trusting an unverified
        // server, so require a fresh pairing instead.
        return Err(TransferError::UntrustedPeer(peer_fp_hex));
    }

    if !ack.client_trusted {
        if !ack.pairing_available {
            return Err(TransferError::PairingUnavailable);
        }
        let code = cfg
            .pairing_code
            .as_deref()
            .ok_or(TransferError::PairingRequired)?;
        tracker.set_phase(TransferPhase::Pairing);
        let proof = pairing_proof(
            code,
            &cfg.identity.fingerprint(),
            &peer_fp,
            &client_nonce,
            &server_nonce,
        )
        .map_err(|e| TransferError::PairingRejected(e.to_string()))?;
        send_message(
            &mut stream,
            &Message::PairRequest(PairRequest {
                device_name: cfg.device_name.clone(),
                fingerprint: cfg.identity.fingerprint_hex(),
                proof: hex::encode(proof),
            }),
            cfg.io_timeout,
            "send pairing request",
        )
        .await?;
        let response =
            recv_message(&mut stream, cfg.approval_timeout, "await pairing approval").await?;
        match response {
            Message::PairAccept(accept) => {
                if !accept.fingerprint.eq_ignore_ascii_case(&peer_fp_hex) {
                    return Err(TransferError::ProtocolViolation(
                        "pairing response fingerprint mismatch".into(),
                    ));
                }
                cfg.trust.trust(&ack.device_name, &peer_fp, unix_now())?;
                info!(peer = %ack.device_name, "paired successfully");
            }
            Message::PairReject(reject) => {
                return Err(TransferError::PairingRejected(reject.reason))
            }
            Message::Error(error) => {
                return Err(TransferError::protocol_error(error.code, error.message))
            }
            other => {
                return Err(TransferError::ProtocolViolation(format!(
                    "expected PAIR_ACCEPT, received {}",
                    other.name()
                )))
            }
        }
    }

    let ready = recv_message(&mut stream, cfg.handshake_timeout, "await session ready").await?;
    match ready {
        Message::SessionReady(ready) => {
            shadow_protocol::validate_transfer_id(&ready.session_id)
                .map_err(|e| TransferError::ProtocolViolation(e.to_string()))?;
        }
        Message::Error(error) => {
            return Err(TransferError::protocol_error(error.code, error.message))
        }
        other => {
            return Err(TransferError::ProtocolViolation(format!(
                "expected SESSION_READY, received {}",
                other.name()
            )))
        }
    }

    Ok(ClientSession {
        stream,
        peer_name: ack.device_name,
        peer_fingerprint: peer_fp_hex,
    })
}

fn validate_entries(
    request: &TransferRequest,
    accept: &TransferAccept,
) -> Result<HashMap<u32, shadow_protocol::ResumeEntry>, TransferError> {
    let mut entries = HashMap::new();
    for entry in &accept.entries {
        let meta = request
            .files
            .iter()
            .find(|meta| meta.file_id == entry.file_id)
            .ok_or_else(|| {
                TransferError::ProtocolViolation("resume entry for unknown file".into())
            })?;
        if entry.offset > meta.size {
            return Err(TransferError::ProtocolViolation(
                "resume offset exceeds file size".into(),
            ));
        }
        if entries.insert(entry.file_id, entry.clone()).is_some() {
            return Err(TransferError::ProtocolViolation(
                "duplicate resume entry".into(),
            ));
        }
    }
    Ok(entries)
}

async fn hash_prefix_into(
    file: &mut File,
    length: u64,
    hasher: &mut Hasher,
) -> Result<blake3::Hash, TransferError> {
    let mut buffer = vec![0u8; HASH_BUFFER];
    let mut remaining = length;
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        let filled = read_full(file, &mut buffer[..want]).await?;
        if filled != want {
            return Err(TransferError::SourceChanged(
                "source ended while validating resume prefix".into(),
            ));
        }
        hasher.update(&buffer[..filled]);
        remaining -= filled as u64;
    }
    Ok(hasher.clone().finalize())
}

async fn read_full(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        let read = file.read(&mut buffer[filled..]).await?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

struct ChunkPlanner {
    policy: ChunkPolicy,
    current: usize,
    window_bytes: u64,
    window_start: Instant,
    last_speed: f64,
}

impl ChunkPlanner {
    fn new(policy: ChunkPolicy) -> Self {
        let current = match policy {
            ChunkPolicy::Adaptive => 1024 * 1024,
            ChunkPolicy::Fixed(size) => size.clamp(MIN_CHUNK, shadow_protocol::MAX_DATA_FRAME),
        };
        Self {
            policy,
            current,
            window_bytes: 0,
            window_start: Instant::now(),
            last_speed: 0.0,
        }
    }

    fn next_chunk(&mut self) -> usize {
        self.current
            .clamp(MIN_CHUNK, shadow_protocol::MAX_DATA_FRAME)
    }

    fn record(&mut self, bytes: u64) {
        let ChunkPolicy::Adaptive = self.policy else {
            return;
        };
        self.window_bytes += bytes;
        let elapsed = self.window_start.elapsed();
        if self.window_bytes < 8 * 1024 * 1024 || elapsed < Duration::from_millis(250) {
            return;
        }
        let speed = self.window_bytes as f64 / elapsed.as_secs_f64();
        if self.last_speed > 0.0 {
            if speed > self.last_speed * 1.05 && self.current < MAX_ADAPTIVE_CHUNK {
                self.current = (self.current * 2).min(MAX_ADAPTIVE_CHUNK);
            } else if speed < self.last_speed * 0.80 && self.current > MIN_ADAPTIVE_CHUNK {
                self.current = (self.current / 2).max(MIN_ADAPTIVE_CHUNK);
            }
        }
        self.last_speed = speed;
        self.window_bytes = 0;
        self.window_start = Instant::now();
    }
}
