use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use shadow_crypto::{
    server_config, unix_now, verify_pairing_proof, Identity, PairingSession, TrustStore,
    ALPN_PROTOCOL, DEFAULT_PAIRING_TTL,
};
use shadow_protocol::{
    ErrorMessage, FileAck, Hello, Message, PairAccept, PairReject, Pong, SessionReady,
    TransferAccept, TransferAck, TransferReject, TransferRequest, PROTOCOL_VERSION,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::sleep;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::error::TransferError;
use crate::path::verify_wire_path;
use crate::progress::{Direction, ProgressEvent, ProgressTracker, TransferPhase};
use crate::session::{
    new_transfer_id, parse_nonce, peer_fingerprint, random_nonce, recv_data, recv_message,
    send_message,
};
use crate::state::{cleanup_stale_states, ReceiveState};

const MAX_PAIRING_FAILURES: usize = 5;
const PAIRING_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const STALE_STATE_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    pub bind: SocketAddr,
    pub destination: PathBuf,
    pub device_name: String,
    pub pairing_ttl: Duration,
    pub allow_new_devices: bool,
    pub max_connections: usize,
    pub approval_timeout: Duration,
    pub io_timeout: Duration,
}

impl ReceiverConfig {
    pub fn new(bind: SocketAddr, destination: PathBuf, device_name: impl Into<String>) -> Self {
        Self {
            bind,
            destination,
            device_name: device_name.into(),
            pairing_ttl: DEFAULT_PAIRING_TTL,
            allow_new_devices: true,
            max_connections: 4,
            approval_timeout: Duration::from_secs(180),
            io_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ApprovalKind {
    Pairing {
        device_name: String,
        fingerprint: String,
    },
    Transfer {
        sender_name: String,
        transfer_id: String,
        file_count: usize,
        total_bytes: u64,
        preview: Vec<String>,
    },
}

impl ApprovalKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pairing { .. } => "Pairing request",
            Self::Transfer { .. } => "Incoming transfer",
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Pairing {
                device_name,
                fingerprint,
            } => format!("{device_name}\nfingerprint {fingerprint}"),
            Self::Transfer {
                sender_name,
                file_count,
                total_bytes,
                ..
            } => format!(
                "{sender_name} wants to send {file_count} file(s), {}",
                crate::util::format_bytes(*total_bytes)
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApprovalNeeded {
    pub id: String,
    pub kind: ApprovalKind,
}

#[derive(Debug, Clone)]
pub enum ReceiverEvent {
    Listening {
        addr: SocketAddr,
    },
    PairingCode {
        code: String,
        expires_in_secs: u32,
    },
    ApprovalNeeded(ApprovalNeeded),
    Progress(ProgressEvent),
    FileReceived {
        transfer_id: String,
        file_name: String,
        destination: PathBuf,
        bytes: u64,
    },
    TransferFinished {
        transfer_id: String,
        files: usize,
        bytes: u64,
        duration_secs: f64,
    },
    TransferFailed {
        transfer_id: Option<String>,
        error: String,
    },
    Stopped,
}

struct Inner {
    config: ReceiverConfig,
    identity: Arc<Identity>,
    trust: Arc<TrustStore>,
    pairing: Mutex<PairingSession>,
    pairing_failures: Mutex<VecDeque<Instant>>,
    approvals: Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    active_transfers: Mutex<HashMap<String, CancellationToken>>,
    events: UnboundedSender<ReceiverEvent>,
    cancel: CancellationToken,
    tls: TlsAcceptor,
    local_addr: SocketAddr,
    connections: Arc<tokio::sync::Semaphore>,
}

pub struct ReceiverHandle {
    inner: Arc<Inner>,
    join: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ReceiverHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    pub fn destination(&self) -> &Path {
        &self.inner.config.destination
    }

    pub fn device_name(&self) -> &str {
        &self.inner.config.device_name
    }

    pub fn pairing_code(&self) -> String {
        let (code, remaining) = self.inner.current_pairing_code();
        let _ = self.inner.events.send(ReceiverEvent::PairingCode {
            code: code.clone(),
            expires_in_secs: remaining,
        });
        code
    }

    pub fn regenerate_pairing_code(&self) -> String {
        {
            let mut guard = lock(&self.inner.pairing);
            let _ = guard.rotate();
        }
        let _ = self.inner.events.send(ReceiverEvent::PairingCode {
            code: self.inner.current_pairing_code().0,
            expires_in_secs: self.inner.current_pairing_code().1,
        });
        self.inner.current_pairing_code().0
    }

    pub fn cancel_transfer(&self, transfer_id: &str) -> bool {
        self.inner.cancel_transfer(transfer_id)
    }

    pub fn active_transfers(&self) -> Vec<String> {
        lock(&self.inner.active_transfers).keys().cloned().collect()
    }

    pub fn respond(&self, approval_id: &str, accepted: bool) -> bool {
        let sender = lock(&self.inner.approvals).remove(approval_id);
        match sender {
            Some(sender) => sender.send(accepted).is_ok(),
            None => false,
        }
    }

    pub async fn shutdown(&self) {
        self.inner.cancel.cancel();
        let join = lock(&self.join).take();
        if let Some(join) = join {
            let _ = tokio::time::timeout(Duration::from_secs(5), join).await;
        }
    }
}

impl Inner {
    fn current_pairing_code(&self) -> (String, u32) {
        let mut guard = lock(&self.pairing);
        if guard.ensure_fresh().is_err() {
            warn!("failed to refresh pairing code");
        }
        (guard.code().to_string(), guard.remaining_secs())
    }

    fn pairing_rate_limited(&self) -> bool {
        let mut failures = lock(&self.pairing_failures);
        let now = Instant::now();
        while let Some(front) = failures.front() {
            if now.duration_since(*front) > PAIRING_FAILURE_WINDOW {
                failures.pop_front();
            } else {
                break;
            }
        }
        failures.len() >= MAX_PAIRING_FAILURES
    }

    fn record_pairing_failure(&self) {
        let mut failures = lock(&self.pairing_failures);
        if failures.len() >= 64 {
            failures.pop_front();
        }
        failures.push_back(Instant::now());
    }

    fn begin_transfer(self: &Arc<Self>, transfer_id: &str) -> Option<TransferGuard> {
        let mut active = lock(&self.active_transfers);
        if active.contains_key(transfer_id) {
            return None;
        }
        let token = CancellationToken::new();
        active.insert(transfer_id.to_string(), token.clone());
        Some(TransferGuard {
            inner: Arc::clone(self),
            transfer_id: transfer_id.to_string(),
            token,
        })
    }

    fn cancel_transfer(&self, transfer_id: &str) -> bool {
        match lock(&self.active_transfers).get(transfer_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

struct TransferGuard {
    inner: Arc<Inner>,
    transfer_id: String,
    token: CancellationToken,
}

impl Drop for TransferGuard {
    fn drop(&mut self) {
        lock(&self.inner.active_transfers).remove(&self.transfer_id);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub async fn start(
    config: ReceiverConfig,
    identity: Arc<Identity>,
    trust: Arc<TrustStore>,
) -> Result<(ReceiverHandle, UnboundedReceiver<ReceiverEvent>), TransferError> {
    tokio::fs::create_dir_all(&config.destination).await?;
    let listener = TcpListener::bind(config.bind).await?;
    let local_addr = listener.local_addr()?;
    let tls = TlsAcceptor::from(server_config(&identity)?);
    let (events, receiver) = unbounded_channel();
    let pairing = PairingSession::new(config.pairing_ttl)
        .map_err(|e| TransferError::Receiver(e.to_string()))?;
    let connections = Arc::new(tokio::sync::Semaphore::new(config.max_connections));

    let destination = config.destination.clone();
    let cleanup =
        tokio::task::spawn_blocking(move || cleanup_stale_states(&destination, STALE_STATE_AGE))
            .await
            .map_err(|e| TransferError::Receiver(format!("state cleanup failed: {e}")))?;
    if cleanup > 0 {
        info!(removed = cleanup, "removed stale partial transfers");
    }

    let inner = Arc::new(Inner {
        config,
        identity,
        trust,
        pairing: Mutex::new(pairing),
        pairing_failures: Mutex::new(VecDeque::new()),
        approvals: Mutex::new(HashMap::new()),
        active_transfers: Mutex::new(HashMap::new()),
        events: events.clone(),
        cancel: CancellationToken::new(),
        tls,
        local_addr,
        connections,
    });

    let handle = ReceiverHandle {
        inner: Arc::clone(&inner),
        join: Mutex::new(None),
    };
    let join = tokio::spawn(accept_loop(Arc::clone(&inner), listener));
    *lock(&handle.join) = Some(join);

    let _ = inner
        .events
        .send(ReceiverEvent::Listening { addr: local_addr });
    let (code, remaining) = inner.current_pairing_code();
    let _ = inner.events.send(ReceiverEvent::PairingCode {
        code,
        expires_in_secs: remaining,
    });
    info!(addr = %local_addr, "receiver listening");
    Ok((handle, receiver))
}

async fn accept_loop(inner: Arc<Inner>, listener: TcpListener) {
    loop {
        tokio::select! {
            _ = inner.cancel.cancelled() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((tcp, addr)) => {
                        let permit = match Arc::clone(&inner.connections).try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(peer = %addr, "connection limit reached; rejecting");
                                continue;
                            }
                        };
                        let task_inner = Arc::clone(&inner);
                        tokio::spawn(async move {
                            let _permit = permit;
                            handle_connection(task_inner, tcp, addr).await;
                        });
                    }
                    Err(err) => {
                        warn!(error = %err, "accept failed");
                        sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
    let _ = inner.events.send(ReceiverEvent::Stopped);
}

async fn handle_connection(inner: Arc<Inner>, tcp: TcpStream, addr: SocketAddr) {
    let _ = tcp.set_nodelay(true);
    crate::session::tune_tcp(&tcp);
    let accept = tokio::time::timeout(Duration::from_secs(15), inner.tls.accept(tcp)).await;
    let mut stream = match accept {
        Ok(Ok(stream)) => stream,
        Ok(Err(err)) => {
            warn!(peer = %addr, error = %err, "TLS handshake failed");
            return;
        }
        Err(_) => {
            warn!(peer = %addr, "TLS handshake timed out");
            return;
        }
    };

    if stream.get_ref().1.alpn_protocol() != Some(ALPN_PROTOCOL) {
        warn!(peer = %addr, "client did not negotiate the Shadow Transfer ALPN");
        return;
    }
    let client_fp = match peer_fingerprint(stream.get_ref().1.peer_certificates()) {
        Ok(fp) => fp,
        Err(err) => {
            warn!(peer = %addr, error = %err, "client certificate missing");
            return;
        }
    };

    let mut transfer_id = None;
    let result = tokio::select! {
        _ = inner.cancel.cancelled() => Ok(()),
        result = run_session(&inner, &mut stream, &client_fp, &mut transfer_id) => result,
    };

    match result {
        Ok(()) => {}
        Err(TransferError::Cancelled) => {
            let message = Message::Error(ErrorMessage {
                code: 409,
                message: "transfer cancelled by receiver".into(),
            });
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                shadow_protocol::write_message(&mut stream, &message),
            )
            .await;
            let _ = inner.events.send(ReceiverEvent::TransferFailed {
                transfer_id: transfer_id.clone(),
                error: "cancelled by receiver".into(),
            });
        }
        Err(err) => {
            warn!(peer = %addr, error = %err, "session ended with error");
            let message = Message::Error(ErrorMessage {
                code: 400,
                message: truncate(&err.to_string(), 1000),
            });
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                shadow_protocol::write_message(&mut stream, &message),
            )
            .await;
            let _ = inner.events.send(ReceiverEvent::TransferFailed {
                transfer_id: transfer_id.clone(),
                error: err.to_string(),
            });
        }
    }
}

async fn run_session(
    inner: &Arc<Inner>,
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    client_fp: &[u8; 32],
    transfer_id_slot: &mut Option<String>,
) -> Result<(), TransferError> {
    let hello = recv_message(stream, Duration::from_secs(15), "await hello").await?;
    let (client_name, client_nonce) = match hello {
        Message::Hello(Hello {
            device_name, nonce, ..
        }) => (device_name, parse_nonce(&nonce)?),
        Message::Error(error) => {
            return Err(TransferError::protocol_error(error.code, error.message))
        }
        other => {
            return Err(TransferError::ProtocolViolation(format!(
                "expected HELLO, received {}",
                other.name()
            )))
        }
    };

    let trusted = inner.trust.is_trusted(client_fp);
    let server_fp = inner.identity.fingerprint();
    let (server_nonce, server_nonce_hex) = random_nonce()?;
    let (pairing_code, pairing_remaining) = inner.current_pairing_code();
    let pairing_available = inner.config.allow_new_devices && !inner.pairing_rate_limited();
    let _ = pairing_code;

    send_message(
        stream,
        &Message::HelloAck(shadow_protocol::HelloAck {
            protocol: PROTOCOL_VERSION,
            device_name: inner.config.device_name.clone(),
            fingerprint: inner.identity.fingerprint_hex(),
            nonce: server_nonce_hex,
            client_trusted: trusted,
            pairing_available,
            pairing_expires_secs: pairing_remaining,
        }),
        Duration::from_secs(15),
        "send hello ack",
    )
    .await?;

    if !trusted {
        if !pairing_available {
            let reason = if inner.config.allow_new_devices {
                "too many pairing attempts; try again later"
            } else {
                "pairing is disabled"
            };
            let _ = send_message(
                stream,
                &Message::PairReject(PairReject {
                    reason: reason.into(),
                }),
                Duration::from_secs(5),
                "reject pairing",
            )
            .await;
            return Err(TransferError::PairingUnavailable);
        }
        complete_pairing(
            inner,
            stream,
            client_fp,
            &client_name,
            &client_nonce,
            &server_nonce,
            &server_fp,
        )
        .await?;
    } else {
        inner.trust.touch(client_fp, unix_now())?;
        info!(device = %client_name, "trusted device connected");
        send_session_ready(stream).await?;
    }

    let request = loop {
        let message =
            recv_message(stream, inner.config.io_timeout, "await transfer request").await?;
        match message {
            Message::TransferRequest(request) => break request,
            Message::Ping(ping) => {
                send_message(
                    stream,
                    &Message::Pong(Pong { nonce: ping.nonce }),
                    Duration::from_secs(5),
                    "send pong",
                )
                .await?;
            }
            Message::Cancel(cancel) => {
                info!(reason = %cancel.reason, "sender cancelled before transfer");
                return Ok(());
            }
            Message::Error(error) => {
                return Err(TransferError::protocol_error(error.code, error.message))
            }
            other => {
                return Err(TransferError::ProtocolViolation(format!(
                    "expected TRANSFER_REQUEST, received {}",
                    other.name()
                )))
            }
        }
    };
    request.validate().map_err(TransferError::Validation)?;
    *transfer_id_slot = Some(request.transfer_id.clone());

    let Some(guard) = inner.begin_transfer(&request.transfer_id) else {
        let _ = send_message(
            stream,
            &Message::TransferReject(TransferReject {
                reason: "this transfer is already in progress".into(),
            }),
            Duration::from_secs(5),
            "reject duplicate transfer",
        )
        .await;
        return Err(TransferError::Receiver("duplicate transfer".into()));
    };

    for meta in &request.files {
        verify_wire_path(&meta.rel_path)
            .map_err(|e| TransferError::ProtocolViolation(e.to_string()))?;
    }

    let approval = ApprovalKind::Transfer {
        sender_name: request.sender_name.clone(),
        transfer_id: request.transfer_id.clone(),
        file_count: request.files.len(),
        total_bytes: request.total_bytes,
        preview: request
            .files
            .iter()
            .take(20)
            .map(|f| f.rel_path.clone())
            .collect(),
    };
    if !request_approval(inner, approval).await? {
        let _ = send_message(
            stream,
            &Message::TransferReject(TransferReject {
                reason: "declined by receiver".into(),
            }),
            Duration::from_secs(5),
            "reject transfer",
        )
        .await;
        info!(transfer = %request.transfer_id, "transfer declined");
        return Ok(());
    }

    let mut state = ReceiveState::open(inner.config.destination.clone(), &request).await?;
    let entries = state.resume_entries();
    send_message(
        stream,
        &Message::TransferAccept(TransferAccept { entries }),
        Duration::from_secs(15),
        "send transfer accept",
    )
    .await?;

    let (progress_tx, mut progress_rx) = unbounded_channel::<ProgressEvent>();
    let events = inner.events.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(event) = progress_rx.recv().await {
            let _ = events.send(ReceiverEvent::Progress(event));
        }
    });
    let mut tracker = ProgressTracker::new(
        progress_tx,
        &request.transfer_id,
        Direction::Receive,
        request.files.len(),
        request.total_bytes,
    );
    tracker.set_phase(TransferPhase::Transferring);

    let started = Instant::now();
    let mut started_files: HashSet<u32> = HashSet::new();
    let result = receive_loop(
        inner,
        stream,
        &request,
        &mut state,
        &mut tracker,
        &guard,
        &mut started_files,
    )
    .await;
    drop(tracker);
    let _ = forwarder.await;

    match result {
        Ok(files) => {
            state.cleanup_after_success();
            let _ = inner.events.send(ReceiverEvent::TransferFinished {
                transfer_id: request.transfer_id.clone(),
                files,
                bytes: request.total_bytes,
                duration_secs: started.elapsed().as_secs_f64(),
            });
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[allow(clippy::too_many_arguments)]
async fn receive_loop(
    inner: &Arc<Inner>,
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    request: &TransferRequest,
    state: &mut ReceiveState,
    tracker: &mut ProgressTracker,
    _guard: &TransferGuard,
    started_files: &mut HashSet<u32>,
) -> Result<usize, TransferError> {
    let token = _guard.token.clone();
    loop {
        let message = tokio::select! {
            _ = token.cancelled() => return Err(TransferError::Cancelled),
            message = recv_message(stream, inner.config.io_timeout, "await transfer message") => message?,
        };
        match message {
            Message::FileStart(start) => {
                let index = file_index(request, start.file_id)?;
                if !started_files.insert(start.file_id) {
                    return Err(TransferError::ProtocolViolation(
                        "duplicate FILE_START for one file".into(),
                    ));
                }
                let meta = &request.files[index];
                tracker.begin_file(index, &meta.rel_path, meta.size, start.offset);
                state.begin_file(start.file_id, start.offset).await?;
            }
            Message::FileChunk(chunk) => {
                let index = file_index(request, chunk.file_id)?;
                if !started_files.contains(&chunk.file_id) {
                    return Err(TransferError::ProtocolViolation(
                        "FILE_CHUNK before FILE_START".into(),
                    ));
                }
                let data = tokio::select! {
                    _ = token.cancelled() => return Err(TransferError::Cancelled),
                    data = recv_data(stream, inner.config.io_timeout, "await chunk") => data?,
                };
                if data.len() != chunk.len as usize {
                    return Err(TransferError::ProtocolViolation(
                        "chunk length does not match header".into(),
                    ));
                }
                let actual = blake3::hash(&data);
                if !chunk.hash.eq_ignore_ascii_case(actual.to_hex().as_str()) {
                    return Err(TransferError::ProtocolViolation(
                        "chunk hash mismatch".into(),
                    ));
                }
                state
                    .write_chunk(chunk.file_id, chunk.offset, &data)
                    .await?;
                tracker.add_bytes(data.len() as u64);
                let _ = index;
            }
            Message::FileComplete(complete) => {
                let index = file_index(request, complete.file_id)?;
                tracker.set_phase(TransferPhase::Verifying);
                let finalized = state
                    .finish_file(complete.file_id, complete.size, &complete.hash)
                    .await;
                match finalized {
                    Ok(finalized) => {
                        send_message(
                            stream,
                            &Message::FileAck(FileAck {
                                file_id: complete.file_id,
                                verified: true,
                                reason: String::new(),
                            }),
                            inner.config.io_timeout,
                            "send file ack",
                        )
                        .await?;
                        tracker.finish_file();
                        tracker.set_phase(TransferPhase::Transferring);
                        let _ = inner.events.send(ReceiverEvent::FileReceived {
                            transfer_id: request.transfer_id.clone(),
                            file_name: request.files[index].rel_path.clone(),
                            destination: finalized.destination,
                            bytes: finalized.bytes,
                        });
                    }
                    Err(err) => {
                        let reason = truncate(&err.to_string(), 200);
                        let _ = send_message(
                            stream,
                            &Message::FileAck(FileAck {
                                file_id: complete.file_id,
                                verified: false,
                                reason: reason.clone(),
                            }),
                            Duration::from_secs(5),
                            "send failed file ack",
                        )
                        .await;
                        return Err(err);
                    }
                }
            }
            Message::TransferComplete(complete) => {
                if complete.transfer_id != request.transfer_id {
                    return Err(TransferError::ProtocolViolation(
                        "transfer id mismatch".into(),
                    ));
                }
                if !state.all_accounted() {
                    return Err(TransferError::State(
                        "sender finished before every file was verified".into(),
                    ));
                }
                send_message(
                    stream,
                    &Message::TransferAck(TransferAck {
                        transfer_id: request.transfer_id.clone(),
                    }),
                    Duration::from_secs(15),
                    "send transfer ack",
                )
                .await?;
                return Ok(request.files.len());
            }
            Message::Cancel(cancel) => {
                info!(reason = %cancel.reason, "sender cancelled transfer");
                return Err(TransferError::TransferRejected(format!(
                    "cancelled by sender: {}",
                    cancel.reason
                )));
            }
            Message::Ping(ping) => {
                send_message(
                    stream,
                    &Message::Pong(Pong { nonce: ping.nonce }),
                    Duration::from_secs(5),
                    "send pong",
                )
                .await?;
            }
            Message::Error(error) => {
                return Err(TransferError::protocol_error(error.code, error.message))
            }
            other => {
                return Err(TransferError::ProtocolViolation(format!(
                    "unexpected message during transfer: {}",
                    other.name()
                )))
            }
        }
    }
}

async fn complete_pairing(
    inner: &Arc<Inner>,
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
    client_fp: &[u8; 32],
    client_name: &str,
    client_nonce: &[u8; 16],
    server_nonce: &[u8; 16],
    server_fp: &[u8; 32],
) -> Result<(), TransferError> {
    let request = recv_message(stream, Duration::from_secs(30), "await pairing request").await?;
    let pair = match request {
        Message::PairRequest(request) => request,
        Message::Cancel(cancel) => {
            info!(reason = %cancel.reason, "client cancelled pairing");
            return Err(TransferError::Cancelled);
        }
        other => {
            return Err(TransferError::ProtocolViolation(format!(
                "expected PAIR_REQUEST, received {}",
                other.name()
            )))
        }
    };

    let claimed = hex::decode(&pair.fingerprint)
        .map_err(|_| TransferError::ProtocolViolation("invalid fingerprint encoding".into()))?;
    if claimed.as_slice() != client_fp {
        inner.record_pairing_failure();
        return Err(TransferError::ProtocolViolation(
            "pairing fingerprint does not match certificate".into(),
        ));
    }

    let valid = {
        let mut guard = lock(&inner.pairing);
        if guard.ensure_fresh().is_err() {
            false
        } else {
            verify_pairing_proof(
                guard.code(),
                client_fp,
                server_fp,
                client_nonce,
                server_nonce,
                &pair.proof,
            )
        }
    };

    if !valid {
        inner.record_pairing_failure();
        // Slow down online guessing. Code entropy plus per-connection limits
        // and the global failure window make brute force impractical.
        sleep(Duration::from_millis(750)).await;
        let _ = send_message(
            stream,
            &Message::PairReject(PairReject {
                reason: "invalid pairing code".into(),
            }),
            Duration::from_secs(5),
            "reject pairing",
        )
        .await;
        return Err(TransferError::PairingRejected(
            "invalid pairing code".into(),
        ));
    }

    let approval = ApprovalKind::Pairing {
        device_name: pair.device_name.clone(),
        fingerprint: hex::encode(client_fp),
    };
    if !request_approval(inner, approval).await? {
        let _ = send_message(
            stream,
            &Message::PairReject(PairReject {
                reason: "pairing declined by receiver".into(),
            }),
            Duration::from_secs(5),
            "reject pairing",
        )
        .await;
        return Err(TransferError::PairingRejected(
            "declined by receiver".into(),
        ));
    }

    inner
        .trust
        .trust(&pair.device_name, client_fp, unix_now())?;
    {
        let mut guard = lock(&inner.pairing);
        let _ = guard.rotate();
    }
    let (code, remaining) = inner.current_pairing_code();
    let _ = inner.events.send(ReceiverEvent::PairingCode {
        code,
        expires_in_secs: remaining,
    });

    send_message(
        stream,
        &Message::PairAccept(PairAccept {
            fingerprint: hex::encode(server_fp),
        }),
        Duration::from_secs(10),
        "accept pairing",
    )
    .await?;
    send_session_ready(stream).await?;
    info!(device = %client_name, "device paired");
    Ok(())
}

async fn send_session_ready(
    stream: &mut tokio_rustls::server::TlsStream<TcpStream>,
) -> Result<(), TransferError> {
    send_message(
        stream,
        &Message::SessionReady(SessionReady {
            session_id: new_transfer_id()?,
        }),
        Duration::from_secs(10),
        "send session ready",
    )
    .await
}

async fn request_approval(inner: &Arc<Inner>, kind: ApprovalKind) -> Result<bool, TransferError> {
    let (_, id) = random_nonce()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    lock(&inner.approvals).insert(id.clone(), tx);
    let _ = inner
        .events
        .send(ReceiverEvent::ApprovalNeeded(ApprovalNeeded {
            id: id.clone(),
            kind,
        }));

    let outcome = tokio::select! {
        _ = inner.cancel.cancelled() => Err(TransferError::Cancelled),
        waited = tokio::time::timeout(inner.config.approval_timeout, rx) => {
            match waited {
                Ok(Ok(accepted)) => Ok(accepted),
                Ok(Err(_)) => Ok(false),
                Err(_) => Ok(false),
            }
        }
    };
    lock(&inner.approvals).remove(&id);
    outcome
}

fn file_index(request: &TransferRequest, file_id: u32) -> Result<usize, TransferError> {
    request
        .files
        .iter()
        .position(|f| f.file_id == file_id)
        .ok_or_else(|| TransferError::ProtocolViolation(format!("unknown file id {file_id}")))
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
