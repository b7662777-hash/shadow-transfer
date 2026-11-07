use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, Color32, Layout, RichText, Rounding, Stroke, Vec2};
use shadow_crypto::{normalize_pairing_code, Identity, TrustStore, TrustedDevice};
use shadow_discovery::{AdvertiseConfig, DiscoveredDevice};
use shadow_transfer::{
    ApprovalKind, ApprovalNeeded, Direction, PairSummary, ProgressEvent, ReceiverConfig,
    ReceiverEvent, ReceiverHandle, SendSummary, SenderConfig, SourceFile, TransferPhase,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::storage::{self, HistoryEntry, Settings};

const BG: Color32 = Color32::from_rgb(8, 9, 11);
const PANEL: Color32 = Color32::from_rgb(13, 14, 18);
const CARD: Color32 = Color32::from_rgb(18, 20, 25);
const BORDER: Color32 = Color32::from_rgb(33, 36, 44);
const TEXT: Color32 = Color32::from_rgb(231, 233, 239);
const MUTED: Color32 = Color32::from_rgb(142, 148, 160);
const ACCENT: Color32 = Color32::from_rgb(143, 126, 255);
const ACCENT_DARK: Color32 = Color32::from_rgb(62, 51, 122);
const GREEN: Color32 = Color32::from_rgb(74, 222, 128);
const RED: Color32 = Color32::from_rgb(248, 113, 113);
const YELLOW: Color32 = Color32::from_rgb(250, 204, 21);

enum AppEvent {
    ReceiverStarted {
        handle: Arc<ReceiverHandle>,
        events: UnboundedReceiver<ReceiverEvent>,
        advertiser: CancellationToken,
    },
    ReceiverStartFailed(String),
    ReceiverEvent(ReceiverEvent),
    SendProgress(ProgressEvent),
    SendDone {
        transfer_id: String,
        peer_addr: SocketAddr,
        result: Result<SendSummary, String>,
        cancelled: bool,
    },
    PairDone {
        addr: SocketAddr,
        result: Result<PairSummary, String>,
    },
    Discovered(Result<Vec<DiscoveredDevice>, String>),
    SourcesAdded(Result<Vec<SourceFile>, String>),
}

#[derive(Clone)]
struct PeerView {
    name: String,
    addr: SocketAddr,
    fingerprint: Option<String>,
    manual: bool,
    last_seen: Option<Instant>,
}

#[derive(Clone)]
struct JobRecord {
    addr: SocketAddr,
    sources: Vec<SourceFile>,
    pairing_code: Option<String>,
}

struct TransferCard {
    id: String,
    direction: Direction,
    peer: String,
    phase: TransferPhase,
    file_name: String,
    file_index: usize,
    file_count: usize,
    file_bytes: u64,
    file_size: u64,
    total_done: u64,
    total_bytes: u64,
    speed_bps: f64,
    avg_bps: f64,
    eta_secs: Option<u64>,
    detail: Option<String>,
    cancel: Option<CancellationToken>,
    job: Option<JobRecord>,
    history_recorded: bool,
}

impl TransferCard {
    fn from_progress(event: &ProgressEvent, peer: String) -> Self {
        Self {
            id: event.transfer_id.clone(),
            direction: event.direction,
            peer,
            phase: event.phase,
            file_name: event.file_name.clone(),
            file_index: event.file_index,
            file_count: event.file_count,
            file_bytes: event.file_bytes,
            file_size: event.file_size,
            total_done: event.total_bytes_done,
            total_bytes: event.total_bytes,
            speed_bps: event.speed_bps,
            avg_bps: event.avg_bps,
            eta_secs: event.eta_secs,
            detail: None,
            cancel: None,
            job: None,
            history_recorded: false,
        }
    }

    fn is_active(&self) -> bool {
        matches!(
            self.phase,
            TransferPhase::Connecting
                | TransferPhase::Pairing
                | TransferPhase::Transferring
                | TransferPhase::Verifying
        )
    }

    fn percent(&self) -> f32 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.total_done as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32
        }
    }
}

struct SendDialog {
    peer: PeerView,
}

struct SettingsDraft {
    device_name: String,
    download_dir: PathBuf,
    receive_port: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Transfers,
    History,
}

struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

pub struct ShadowApp {
    runtime: tokio::runtime::Runtime,
    data_dir: PathBuf,
    identity: Arc<Identity>,
    trust: Arc<TrustStore>,
    settings: Settings,
    history: Vec<HistoryEntry>,
    events_tx: UnboundedSender<AppEvent>,
    events_rx: UnboundedReceiver<AppEvent>,
    peers: Vec<PeerView>,
    selected_addr: Option<SocketAddr>,
    manual_host: String,
    manual_port: String,
    auto_code: String,
    manual_code: String,
    pairing_in_progress: Option<SocketAddr>,
    pair_error: Option<(SocketAddr, String)>,
    discovery_running: bool,
    discovery_cancel: Option<CancellationToken>,
    next_discovery_at: Instant,
    sources: Vec<SourceFile>,
    sources_loading: bool,
    total_bytes: u64,
    receive_requested: bool,
    receiver_starting: bool,
    receiver: Option<Arc<ReceiverHandle>>,
    receiver_addr: Option<SocketAddr>,
    advertiser_cancel: Option<CancellationToken>,
    pairing_code: String,
    pairing_expires: u32,
    pairing_expires_at: Option<Instant>,
    approvals: VecDeque<ApprovalNeeded>,
    peer_names: HashMap<String, String>,
    cards: Vec<TransferCard>,
    send_dialog: Option<SendDialog>,
    settings_open: bool,
    settings_draft: Option<SettingsDraft>,
    tab: Tab,
    toast: Option<Toast>,
    auto_approve: bool,
    demo_autosend: Option<(SocketAddr, String)>,
    demo_autosend_started: bool,
}

impl ShadowApp {
    fn try_new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        let data_dir = storage::data_dir()?;
        let identity =
            Arc::new(Identity::load_or_create(&data_dir).map_err(|e| format!("identity: {e}"))?);
        let trust = Arc::new(
            TrustStore::load(data_dir.join("trusted.json"))
                .map_err(|e| format!("trust store: {e}"))?,
        );
        let fallback_download = dirs::download_dir()
            .map(|dir| dir.join("ShadowTransfer"))
            .unwrap_or_else(|| data_dir.join("Received"));
        let mut warnings = Vec::new();
        let settings =
            match Settings::load(&data_dir.join("settings.json"), fallback_download.clone()) {
                Ok(settings) => settings,
                Err(err) => {
                    warnings.push(err);
                    Settings::default().with_default_download(fallback_download)
                }
            };
        let history = match storage::load_history(&data_dir.join("history.json")) {
            Ok(history) => history,
            Err(err) => {
                warnings.push(err);
                Vec::new()
            }
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("tokio runtime: {e}"))?;
        let (events_tx, events_rx) = unbounded_channel();
        let mut app = Self {
            runtime,
            data_dir,
            identity,
            trust,
            manual_port: "47831".to_string(),
            settings,
            history,
            events_tx,
            events_rx,
            peers: Vec::new(),
            selected_addr: None,
            manual_host: String::new(),
            auto_code: String::new(),
            manual_code: String::new(),
            pairing_in_progress: None,
            pair_error: None,
            discovery_running: false,
            discovery_cancel: None,
            next_discovery_at: Instant::now(),
            sources: Vec::new(),
            sources_loading: false,
            total_bytes: 0,
            receive_requested: false,
            receiver_starting: false,
            receiver: None,
            receiver_addr: None,
            advertiser_cancel: None,
            pairing_code: String::new(),
            pairing_expires: 0,
            pairing_expires_at: None,
            approvals: VecDeque::new(),
            peer_names: HashMap::new(),
            cards: Vec::new(),
            send_dialog: None,
            settings_open: false,
            settings_draft: None,
            tab: Tab::Transfers,
            toast: None,
            auto_approve: cfg!(debug_assertions)
                && std::env::var_os("SHADOW_UI_AUTO_APPROVE").is_some(),
            demo_autosend: if cfg!(debug_assertions) {
                match (
                    std::env::var("SHADOW_UI_AUTOSEND_PEER"),
                    std::env::var("SHADOW_UI_AUTOSEND_CODE"),
                ) {
                    (Ok(peer), Ok(code)) => peer.parse().ok().map(|addr| (addr, code)),
                    _ => None,
                }
            } else {
                None
            },
            demo_autosend_started: false,
        };
        app.manual_port = app.settings.receive_port.to_string();
        if let Some(warning) = warnings.first() {
            app.set_toast(warning.clone(), true);
        }
        let _ = cc;
        if std::env::var_os("SHADOW_UI_AUTOSTART_RECEIVE").is_some() {
            app.start_receiving();
        }
        if cfg!(debug_assertions) {
            if let Ok(path) = std::env::var("SHADOW_UI_DEMO_FILES") {
                app.add_paths(vec![PathBuf::from(path)]);
            }
        }
        app.start_discovery();
        Ok(app)
    }

    fn set_toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    fn start_discovery(&mut self) {
        if self.discovery_running {
            return;
        }
        self.discovery_running = true;
        self.next_discovery_at = Instant::now() + Duration::from_secs(8);
        let cancel = CancellationToken::new();
        if let Some(previous) = self.discovery_cancel.replace(cancel.clone()) {
            previous.cancel();
        }
        let tx = self.events_tx.clone();
        self.runtime.spawn(async move {
            let result = shadow_discovery::discover_for(Duration::from_secs(3), cancel)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Discovered(result));
        });
    }

    fn start_receiving(&mut self) {
        if self.receiver.is_some() || self.receiver_starting {
            self.receive_requested = true;
            return;
        }
        if let Err(err) = self.settings.validate() {
            self.receive_requested = false;
            self.set_toast(err, true);
            return;
        }
        self.receive_requested = true;
        self.receiver_starting = true;
        let config = ReceiverConfig::new(
            SocketAddr::from(([0, 0, 0, 0], self.settings.receive_port)),
            self.settings.download_dir.clone(),
            self.settings.device_name.clone(),
        );
        let identity = Arc::clone(&self.identity);
        let trust = Arc::clone(&self.trust);
        let tx = self.events_tx.clone();
        let fingerprint = identity.fingerprint_hex();
        self.runtime.spawn(async move {
            match shadow_transfer::start(config, Arc::clone(&identity), trust).await {
                Ok((handle, events)) => {
                    let advertiser = CancellationToken::new();
                    let advertise_cancel = advertiser.clone();
                    let advertise = AdvertiseConfig::new(
                        handle.device_name().to_string(),
                        handle.local_addr().port(),
                        fingerprint,
                    );
                    tokio::spawn(async move {
                        let _ = shadow_discovery::advertise(advertise, advertise_cancel).await;
                    });
                    let _ = tx.send(AppEvent::ReceiverStarted {
                        handle: Arc::new(handle),
                        events,
                        advertiser,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::ReceiverStartFailed(err.to_string()));
                }
            }
        });
    }

    fn stop_receiving(&mut self) {
        self.receive_requested = false;
        self.receiver_starting = false;
        if let Some(cancel) = self.advertiser_cancel.take() {
            cancel.cancel();
        }
        if let Some(handle) = self.receiver.take() {
            for approval in self.approvals.drain(..) {
                let _ = handle.respond(&approval.id, false);
            }
            self.runtime.spawn(async move {
                handle.shutdown().await;
            });
        }
        self.pairing_code.clear();
        self.pairing_expires = 0;
        self.pairing_expires_at = None;
        self.receiver_addr = None;
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.sources_loading = true;
        let tx = self.events_tx.clone();
        self.runtime.spawn(async move {
            let result = shadow_transfer::collect_sources(paths)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::SourcesAdded(result));
        });
    }

    fn merge_sources(&mut self, new_sources: Vec<SourceFile>) {
        let mut skipped = 0;
        for source in new_sources {
            if self.sources.iter().any(|s| s.rel_path == source.rel_path) {
                skipped += 1;
            } else {
                self.sources.push(source);
            }
        }
        self.sources.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        self.total_bytes = self.sources.iter().map(|s| s.size).sum();
        if skipped > 0 {
            self.set_toast(
                format!("{skipped} item(s) ignored because the destination name already exists"),
                true,
            );
        }
    }

    fn selected_peer(&self) -> Option<PeerView> {
        self.selected_addr
            .and_then(|addr| self.peers.iter().find(|p| p.addr == addr).cloned())
    }

    fn open_send_dialog(&mut self) {
        let Some(peer) = self.selected_peer() else {
            self.set_toast("Select a device first", true);
            return;
        };
        if self.sources.is_empty() {
            self.set_toast("Add files or a folder first", true);
            return;
        }
        if !peer_trusted(&self.trust, &peer) {
            self.set_toast(
                "Pair this device first: select it, enter the code shown on the tablet, and press Pair.",
                true,
            );
            return;
        }
        self.send_dialog = Some(SendDialog { peer });
    }

    fn begin_send(&mut self, peer: PeerView, code: String) {
        if self.sources.is_empty() {
            self.set_toast("No files selected", true);
            return;
        }
        let transfer_id = match shadow_transfer::generate_transfer_id() {
            Ok(id) => id,
            Err(err) => {
                self.set_toast(err.to_string(), true);
                return;
            }
        };
        let job = JobRecord {
            addr: peer.addr,
            sources: self.sources.clone(),
            pairing_code: if code.trim().is_empty() {
                None
            } else {
                Some(code.trim().to_string())
            },
        };
        let token = CancellationToken::new();
        self.cards.insert(
            0,
            TransferCard {
                id: transfer_id.clone(),
                direction: Direction::Send,
                peer: peer.name.clone(),
                phase: TransferPhase::Connecting,
                file_name: String::new(),
                file_index: 0,
                file_count: self.sources.len(),
                file_bytes: 0,
                file_size: 0,
                total_done: 0,
                total_bytes: self.total_bytes,
                speed_bps: 0.0,
                avg_bps: 0.0,
                eta_secs: None,
                detail: None,
                cancel: Some(token.clone()),
                job: Some(job.clone()),
                history_recorded: false,
            },
        );
        self.spawn_send(transfer_id, job, token);
    }

    fn spawn_send(&mut self, transfer_id: String, job: JobRecord, token: CancellationToken) {
        let identity = Arc::clone(&self.identity);
        let trust = Arc::clone(&self.trust);
        let device_name = self.settings.device_name.clone();
        let tx = self.events_tx.clone();
        let peer_addr = job.addr;
        self.runtime.spawn(async move {
            let mut config = SenderConfig::new(job.addr, identity, trust, device_name);
            config.pairing_code = job.pairing_code.clone();
            let (progress_tx, mut progress_rx) = unbounded_channel::<ProgressEvent>();
            let forward_tx = tx.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(event) = progress_rx.recv().await {
                    if forward_tx.send(AppEvent::SendProgress(event)).is_err() {
                        break;
                    }
                }
            });
            let result = shadow_transfer::send_sources(
                &config,
                Some(transfer_id.clone()),
                job.sources.clone(),
                progress_tx,
                token.clone(),
            )
            .await;
            let cancelled = token.is_cancelled();
            let _ = forwarder.await;
            let _ = tx.send(AppEvent::SendDone {
                transfer_id,
                peer_addr,
                result: result.map_err(|e| e.to_string()),
                cancelled,
            });
        });
    }

    fn retry_card(&mut self, index: usize) {
        let Some(card) = self.cards.get_mut(index) else {
            return;
        };
        let Some(job) = card.job.clone() else {
            return;
        };
        let token = CancellationToken::new();
        card.phase = TransferPhase::Connecting;
        card.detail = None;
        card.total_done = 0;
        card.speed_bps = 0.0;
        card.avg_bps = 0.0;
        card.eta_secs = None;
        card.cancel = Some(token.clone());
        card.history_recorded = false;
        let id = card.id.clone();
        self.spawn_send(id, job, token);
    }

    fn handle_app_event(&mut self, ctx: &egui::Context, event: AppEvent) {
        match event {
            AppEvent::ReceiverStarted {
                handle,
                events,
                advertiser,
            } => {
                self.receiver_starting = false;
                self.receive_requested = true;
                self.receiver_addr = Some(handle.local_addr());
                self.advertiser_cancel = Some(advertiser);
                self.receiver = Some(Arc::clone(&handle));
                self.set_toast(
                    "Receive mode active. Enter the pairing code on the sending device.",
                    false,
                );
                let tx = self.events_tx.clone();
                self.runtime.spawn(async move {
                    let mut events = events;
                    while let Some(event) = events.recv().await {
                        if tx.send(AppEvent::ReceiverEvent(event)).is_err() {
                            break;
                        }
                    }
                });
                ctx.request_repaint();
            }
            AppEvent::ReceiverStartFailed(err) => {
                self.receiver_starting = false;
                self.receive_requested = false;
                self.set_toast(format!("Cannot start receive mode: {err}"), true);
            }
            AppEvent::ReceiverEvent(event) => self.handle_receiver_event(event),
            AppEvent::SendProgress(event) => self.apply_progress(event),
            AppEvent::SendDone {
                transfer_id,
                peer_addr,
                result,
                cancelled,
            } => self.handle_send_done(transfer_id, peer_addr, result, cancelled),
            AppEvent::PairDone { addr, result } => self.handle_pair_done(addr, result),
            AppEvent::Discovered(result) => {
                self.discovery_running = false;
                self.next_discovery_at = Instant::now() + Duration::from_secs(6);
                match result {
                    Ok(devices) => {
                        self.merge_discovered(devices);
                    }
                    Err(err) => self.set_toast(format!("Discovery failed: {err}"), true),
                }
            }
            AppEvent::SourcesAdded(result) => {
                self.sources_loading = false;
                match result {
                    Ok(sources) => self.merge_sources(sources),
                    Err(err) => self.set_toast(format!("Cannot add selection: {err}"), true),
                }
            }
        }
    }

    fn merge_discovered(&mut self, devices: Vec<DiscoveredDevice>) {
        let now = Instant::now();
        let manual: Vec<PeerView> = self
            .peers
            .iter()
            .filter(|peer| peer.manual)
            .cloned()
            .collect();

        let mut auto: Vec<PeerView> = devices
            .iter()
            .map(|device| PeerView {
                name: device.name.clone(),
                addr: device.addr,
                fingerprint: Some(device.fingerprint.clone()),
                manual: false,
                last_seen: Some(now),
            })
            .collect();

        // Keep recently seen devices as offline entries instead of making them
        // vanish after one missed beacon.
        for old in self.peers.iter().filter(|peer| !peer.manual) {
            if devices.iter().any(|device| device.addr == old.addr) {
                continue;
            }
            if old
                .last_seen
                .map(|seen| now.duration_since(seen) < Duration::from_secs(300))
                .unwrap_or(false)
            {
                auto.push(old.clone());
            }
        }

        auto.sort_by(|a, b| {
            let a_online = peer_is_online(a);
            let b_online = peer_is_online(b);
            let a_trusted = peer_trusted(&self.trust, a);
            let b_trusted = peer_trusted(&self.trust, b);
            b_online
                .cmp(&a_online)
                .then_with(|| b_trusted.cmp(&a_trusted))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.peers = auto;
        self.peers.extend(manual);

        let selection_valid = self
            .selected_addr
            .and_then(|addr| self.peers.iter().find(|peer| peer.addr == addr))
            .map(|peer| peer_is_online(peer) || peer.manual)
            .unwrap_or(false);
        if !selection_valid {
            self.selected_addr = self
                .peers
                .iter()
                .find(|peer| peer_is_online(peer) && peer_trusted(&self.trust, peer))
                .or_else(|| self.peers.iter().find(|peer| peer_is_online(peer)))
                .or_else(|| self.peers.first())
                .map(|peer| peer.addr);
        }
    }

    fn handle_receiver_event(&mut self, event: ReceiverEvent) {
        match event {
            ReceiverEvent::Listening { addr } => {
                self.receiver_addr = Some(addr);
            }
            ReceiverEvent::PairingCode {
                code,
                expires_in_secs,
            } => {
                self.pairing_code = code;
                self.pairing_expires = expires_in_secs;
                self.pairing_expires_at =
                    Some(Instant::now() + Duration::from_secs(expires_in_secs as u64));
            }
            ReceiverEvent::ApprovalNeeded(approval) => {
                if let ApprovalKind::Transfer {
                    transfer_id,
                    sender_name,
                    ..
                } = &approval.kind
                {
                    self.peer_names
                        .insert(transfer_id.clone(), sender_name.clone());
                }
                if self.auto_approve {
                    if let Some(handle) = &self.receiver {
                        let _ = handle.respond(&approval.id, true);
                    }
                } else {
                    self.approvals.push_back(approval);
                }
            }
            ReceiverEvent::Progress(event) => self.apply_progress(event),
            ReceiverEvent::FileReceived { .. } => {}
            ReceiverEvent::TransferFinished {
                transfer_id,
                files,
                bytes,
                duration_secs,
            } => {
                let peer = self
                    .peer_names
                    .get(&transfer_id)
                    .cloned()
                    .unwrap_or_else(|| "Incoming device".to_string());
                let mut record = false;
                if let Some(card) = self.card_mut(&transfer_id) {
                    card.phase = TransferPhase::Completed;
                    card.total_done = bytes;
                    card.total_bytes = bytes;
                    card.file_count = files;
                    card.cancel = None;
                    card.detail = Some(format!("BLAKE3 verified · {duration_secs:.1}s"));
                    if !card.history_recorded {
                        card.history_recorded = true;
                        record = true;
                    }
                } else {
                    self.cards.insert(
                        0,
                        TransferCard {
                            id: transfer_id.clone(),
                            direction: Direction::Receive,
                            peer: peer.clone(),
                            phase: TransferPhase::Completed,
                            file_name: String::new(),
                            file_index: 0,
                            file_count: files,
                            file_bytes: 0,
                            file_size: 0,
                            total_done: bytes,
                            total_bytes: bytes,
                            speed_bps: 0.0,
                            avg_bps: 0.0,
                            eta_secs: None,
                            detail: Some("BLAKE3 verified".into()),
                            cancel: None,
                            job: None,
                            history_recorded: true,
                        },
                    );
                    record = true;
                }
                if record {
                    self.record_history(
                        "Received",
                        peer,
                        files,
                        bytes,
                        "Verified",
                        String::new(),
                        transfer_id,
                    );
                }
                self.set_toast(
                    format!(
                        "Received {files} file(s), {}",
                        shadow_transfer::util::format_bytes(bytes)
                    ),
                    false,
                );
            }
            ReceiverEvent::TransferFailed { transfer_id, error } => {
                if let Some(id) = transfer_id {
                    let cancelled = error.to_lowercase().contains("cancelled");
                    let peer = self
                        .peer_names
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| "Incoming device".to_string());
                    let mut record = false;
                    let files;
                    let bytes;
                    if let Some(card) = self.card_mut(&id) {
                        if card.phase != TransferPhase::Completed {
                            card.phase = if cancelled {
                                TransferPhase::Cancelled
                            } else {
                                TransferPhase::Failed
                            };
                            card.detail = Some(error.clone());
                            card.cancel = None;
                            if !card.history_recorded {
                                card.history_recorded = true;
                                record = true;
                            }
                        }
                        files = card.file_count;
                        bytes = card.total_done;
                    } else {
                        files = 0;
                        bytes = 0;
                        record = false;
                    }
                    if record {
                        self.record_history(
                            "Received",
                            peer,
                            files,
                            bytes,
                            if cancelled { "Cancelled" } else { "Failed" },
                            error.clone(),
                            id,
                        );
                    }
                    if !cancelled {
                        self.set_toast(error, true);
                    }
                } else if !error.is_empty() {
                    self.set_toast(error, true);
                }
            }
            ReceiverEvent::Stopped => {
                self.receiver = None;
                self.receive_requested = false;
                self.receiver_starting = false;
                self.pairing_code.clear();
            }
        }
    }

    fn apply_progress(&mut self, event: ProgressEvent) {
        let position = self
            .cards
            .iter()
            .position(|card| card.id == event.transfer_id);
        let index = match position {
            Some(index) => index,
            None => {
                let peer = if event.direction == Direction::Receive {
                    self.peer_names
                        .get(&event.transfer_id)
                        .cloned()
                        .unwrap_or_else(|| "Incoming device".to_string())
                } else {
                    self.selected_peer()
                        .map(|p| p.name)
                        .unwrap_or_else(|| "Outgoing".to_string())
                };
                self.cards
                    .insert(0, TransferCard::from_progress(&event, peer));
                0
            }
        };
        let card = &mut self.cards[index];
        card.phase = event.phase;
        card.file_name = event.file_name;
        card.file_index = event.file_index;
        card.file_count = event.file_count;
        card.file_bytes = event.file_bytes;
        card.file_size = event.file_size;
        card.total_done = event.total_bytes_done;
        card.total_bytes = event.total_bytes;
        card.speed_bps = event.speed_bps;
        card.avg_bps = event.avg_bps;
        card.eta_secs = event.eta_secs;
    }

    fn handle_send_done(
        &mut self,
        transfer_id: String,
        peer_addr: SocketAddr,
        result: Result<SendSummary, String>,
        cancelled: bool,
    ) {
        let mut history: Option<(String, String, usize, u64, String, String)> = None;
        let mut toast: Option<(String, bool)> = None;
        if let Some(card) = self.card_mut(&transfer_id) {
            card.cancel = None;
            match result {
                Ok(summary) => {
                    card.phase = TransferPhase::Completed;
                    card.total_done = summary.bytes_sent + summary.bytes_reused;
                    card.total_bytes = card.total_bytes.max(card.total_done);
                    card.speed_bps = 0.0;
                    card.avg_bps = summary.avg_bps;
                    card.eta_secs = Some(0);
                    card.detail = Some("BLAKE3 verified on receiver".into());
                    card.job = None;
                    if !card.history_recorded {
                        card.history_recorded = true;
                        history = Some((
                            "Sent".into(),
                            summary.peer_name.clone(),
                            summary.files.len(),
                            summary.bytes_sent + summary.bytes_reused,
                            "Verified".into(),
                            String::new(),
                        ));
                    }
                    if let Some(peer) = self.peers.iter_mut().find(|p| p.addr == peer_addr) {
                        if peer.fingerprint.is_none() {
                            peer.fingerprint = Some(summary.peer_fingerprint.clone());
                        }
                        peer.name = summary.peer_name.clone();
                    }
                    toast = Some((
                        format!(
                            "Sent {} file(s), {}",
                            summary.files.len(),
                            shadow_transfer::util::format_bytes(summary.bytes_sent)
                        ),
                        false,
                    ));
                }
                Err(err) => {
                    card.phase = if cancelled {
                        TransferPhase::Cancelled
                    } else {
                        TransferPhase::Failed
                    };
                    card.detail = Some(err.clone());
                    if !card.history_recorded {
                        card.history_recorded = true;
                        history = Some((
                            "Sent".into(),
                            card.peer.clone(),
                            card.file_count,
                            card.total_done,
                            if cancelled {
                                "Cancelled".into()
                            } else {
                                "Failed".into()
                            },
                            err.clone(),
                        ));
                    }
                    toast = Some((err, !cancelled));
                }
            }
        }
        if let Some((direction, peer, files, bytes, status, detail)) = history {
            self.record_history(&direction, peer, files, bytes, &status, detail, transfer_id);
        }
        if let Some((text, error)) = toast {
            self.set_toast(text, error);
        }
    }

    fn card_mut(&mut self, id: &str) -> Option<&mut TransferCard> {
        self.cards.iter_mut().find(|card| card.id == id)
    }

    #[allow(clippy::too_many_arguments)]
    fn record_history(
        &mut self,
        direction: &str,
        peer: String,
        files: usize,
        bytes: u64,
        status: &str,
        detail: String,
        transfer_id: String,
    ) {
        self.history.insert(
            0,
            HistoryEntry {
                at: storage::now_unix(),
                direction: direction.to_string(),
                peer,
                files,
                bytes,
                status: status.to_string(),
                detail,
                transfer_id,
            },
        );
        if self.history.len() > 200 {
            self.history.truncate(200);
        }
        if let Err(err) = storage::save_history(&self.data_dir.join("history.json"), &self.history)
        {
            self.set_toast(format!("Could not save history: {err}"), true);
        }
    }

    fn spawn_pair(&mut self, addr: SocketAddr, code: String) {
        if self.pairing_in_progress.is_some() {
            self.set_toast("A pairing attempt is already in progress", true);
            return;
        }
        let code = match normalize_pairing_code(&code) {
            Ok(code) => code,
            Err(_) => {
                self.set_toast(
                    "Enter the 8-character code shown on the tablet (letters/numbers; no O, 0, I or 1).",
                    true,
                );
                return;
            }
        };
        self.pairing_in_progress = Some(addr);
        self.pair_error = None;
        let identity = Arc::clone(&self.identity);
        let trust = Arc::clone(&self.trust);
        let device_name = self.settings.device_name.clone();
        let tx = self.events_tx.clone();
        self.runtime.spawn(async move {
            let mut config = SenderConfig::new(addr, identity, trust, device_name);
            config.pairing_code = Some(code);
            config.approval_timeout = Duration::from_secs(180);
            let result = shadow_transfer::pair_with(&config)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::PairDone { addr, result });
        });
    }

    fn handle_pair_done(&mut self, addr: SocketAddr, result: Result<PairSummary, String>) {
        self.pairing_in_progress = None;
        match result {
            Ok(summary) => {
                self.pair_error = None;
                self.auto_code.clear();
                self.manual_code.clear();
                if let Some(peer) = self.peers.iter_mut().find(|peer| peer.addr == addr) {
                    peer.name = summary.peer_name.clone();
                    if peer.fingerprint.is_none() {
                        peer.fingerprint = Some(summary.peer_fingerprint.clone());
                    }
                }
                self.selected_addr = Some(addr);
                self.set_toast(
                    format!(
                        "Paired with {}. Choose files, then press Send.",
                        summary.peer_name
                    ),
                    false,
                );
            }
            Err(err) => {
                self.pair_error = Some((addr, err.clone()));
                self.set_toast(format!("Pairing failed: {err}"), true);
            }
        }
    }

    fn connect_manual(&mut self) {
        let host = self.manual_host.trim().to_string();
        if host.is_empty() {
            self.set_toast("Enter the tablet's IP address", true);
            return;
        }
        let port = match self.manual_port.trim().parse::<u16>() {
            Ok(port) if port >= 1024 => port,
            _ => {
                self.set_toast(
                    "Enter a valid port (1024-65535). The pairing code belongs in its own field.",
                    true,
                );
                return;
            }
        };
        let addr = if let Ok(addr) = host.parse::<SocketAddr>() {
            addr
        } else if let Ok(ip) = host.parse::<IpAddr>() {
            SocketAddr::new(ip, port)
        } else if let Ok(addr) = format!("{host}:{port}").parse::<SocketAddr>() {
            addr
        } else {
            self.set_toast(
                "Invalid IP address. The pairing code belongs in its own field.",
                true,
            );
            return;
        };
        if !self.peers.iter().any(|peer| peer.addr == addr) {
            self.peers.push(PeerView {
                name: format!("Manual {addr}"),
                addr,
                fingerprint: None,
                manual: true,
                last_seen: None,
            });
        }
        self.selected_addr = Some(addr);
        self.spawn_pair(addr, self.manual_code.clone());
    }

    fn open_settings(&mut self) {
        self.settings_draft = Some(SettingsDraft {
            device_name: self.settings.device_name.clone(),
            download_dir: self.settings.download_dir.clone(),
            receive_port: self.settings.receive_port,
        });
        self.settings_open = true;
    }

    fn save_settings(&mut self) {
        let Some(draft) = self.settings_draft.as_ref() else {
            return;
        };
        let new_settings = Settings {
            device_name: draft.device_name.trim().to_string(),
            download_dir: draft.download_dir.clone(),
            receive_port: draft.receive_port,
        };
        if let Err(err) = new_settings.validate() {
            self.set_toast(err, true);
            return;
        }
        if let Err(err) = new_settings.save(&self.data_dir.join("settings.json")) {
            self.set_toast(err, true);
            return;
        }
        let needs_restart = self.receiver.is_some()
            && (new_settings.device_name != self.settings.device_name
                || new_settings.receive_port != self.settings.receive_port
                || new_settings.download_dir != self.settings.download_dir);
        self.settings = new_settings;
        self.manual_port = self.settings.receive_port.to_string();
        self.settings_open = false;
        self.settings_draft = None;
        if needs_restart {
            self.stop_receiving();
            self.set_toast(
                "Settings saved. Enable receive mode again to apply them.",
                false,
            );
        } else {
            self.set_toast("Settings saved", false);
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_paths(dropped);
        }
    }

    fn ongoing(&self) -> bool {
        self.receive_requested
            || self.receiver_starting
            || self.discovery_running
            || self.sources_loading
            || self.cards.iter().any(TransferCard::is_active)
    }

    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::none()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(20.0, 14.0))
                    .stroke(Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("SHADOW TRANSFER")
                                .size(21.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new("Transfer Without Limits.")
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Settings").clicked() {
                            self.open_settings();
                        }
                        let mut want = self.receive_requested;
                        let label = if self.receiver_starting {
                            "Starting…"
                        } else if self.receive_requested {
                            "Receive: ON"
                        } else {
                            "Receive: OFF"
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(label)
                                        .strong()
                                        .color(if self.receive_requested { GREEN } else { MUTED }),
                                )
                                .fill(if self.receive_requested {
                                    ACCENT_DARK
                                } else {
                                    CARD
                                })
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    if self.receive_requested {
                                        GREEN
                                    } else {
                                        BORDER
                                    },
                                )),
                            )
                            .clicked()
                        {
                            want = !want;
                            if want {
                                self.start_receiving();
                            } else {
                                self.stop_receiving();
                            }
                        }
                        let (status, color) = if self.receiver.is_some() {
                            ("Listening", GREEN)
                        } else if self.receiver_starting {
                            ("Starting", YELLOW)
                        } else {
                            ("Offline", MUTED)
                        };
                        ui.label(RichText::new(status).small().color(color));
                    });
                });
            });
    }

    fn side_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(372.0)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(16.0, 12.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.devices_section(ui);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        self.files_section(ui);
                        if self.receive_requested || self.receiver.is_some() {
                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(10.0);
                            self.receive_section(ui);
                        }
                    });
            });
    }

    fn devices_section(&mut self, ui: &mut egui::Ui) {
        self.auto_discovery_card(ui);
        ui.add_space(10.0);
        self.manual_connection_card(ui);
        if let Some((_, error)) = &self.pair_error {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("Pairing failed: {error}"))
                    .small()
                    .color(RED),
            );
        }
    }

    fn auto_discovery_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(Rounding::same(10.0))
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("AUTO DISCOVERY")
                            .size(11.0)
                            .strong()
                            .color(MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.discovery_running, egui::Button::new("Scan"))
                            .clicked()
                        {
                            self.start_discovery();
                        }
                    });
                });
                if self.discovery_running {
                    ui.label(
                        RichText::new("Scanning local network...")
                            .small()
                            .color(MUTED),
                    );
                }
                let peers: Vec<PeerView> = self
                    .peers
                    .iter()
                    .filter(|peer| !peer.manual)
                    .cloned()
                    .collect();
                if peers.is_empty() {
                    ui.label(
                        RichText::new(
                            "No automatic devices yet. Turn on Receive on the tablet and scan.",
                        )
                        .small()
                        .color(MUTED),
                    );
                    if !self.discovery_running
                        && ui
                            .add_sized(
                                [ui.available_width(), 30.0],
                                egui::Button::new(RichText::new("Scan for devices").strong()),
                            )
                            .clicked()
                    {
                        self.start_discovery();
                    }
                }
                let mut select: Option<SocketAddr> = None;
                for peer in &peers {
                    let selected = self.selected_addr == Some(peer.addr);
                    let trusted = peer_trusted(&self.trust, peer);
                    egui::Frame::none()
                        .fill(if selected { ACCENT_DARK } else { CARD })
                        .stroke(Stroke::new(1.0_f32, if selected { ACCENT } else { BORDER }))
                        .rounding(Rounding::same(8.0))
                        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(
                                        selected,
                                        RichText::new(&peer.name).strong().color(TEXT),
                                    )
                                    .clicked()
                                {
                                    select = Some(peer.addr);
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(if trusted {
                                            "trusted"
                                        } else {
                                            "code needed"
                                        })
                                        .small()
                                        .color(if trusted { GREEN } else { YELLOW }),
                                    );
                                });
                            });
                            let detail = match &peer.fingerprint {
                                Some(fingerprint) => format!(
                                    "{} · {}",
                                    peer.addr,
                                    shadow_transfer::util::short_fingerprint(fingerprint)
                                ),
                                None => peer.addr.to_string(),
                            };
                            let presence = if peer.manual {
                                "manual".to_string()
                            } else if peer_is_online(peer) {
                                "online".to_string()
                            } else if let Some(seen) = peer.last_seen {
                                format!("offline · seen {}s ago", seen.elapsed().as_secs())
                            } else {
                                "offline".to_string()
                            };
                            ui.label(
                                RichText::new(format!("{detail} · {presence}"))
                                    .small()
                                    .color(if peer_is_online(peer) || peer.manual {
                                        MUTED
                                    } else {
                                        YELLOW
                                    }),
                            );
                        });
                    ui.add_space(6.0);
                }
                if let Some(addr) = select {
                    self.selected_addr = Some(addr);
                }

                let selected = self.selected_peer().filter(|peer| !peer.manual);
                if let Some(peer) = selected {
                    let trusted = peer_trusted(&self.trust, &peer);
                    ui.add_space(2.0);
                    if trusted {
                        ui.label(
                            RichText::new(format!("Paired with {}. Ready to send.", peer.name))
                                .small()
                                .color(GREEN),
                        );
                    } else {
                        ui.label(
                            RichText::new("PAIRING CODE (first connection)")
                                .size(10.0)
                                .strong()
                                .color(MUTED),
                        );
                        let code_response = ui.add(
                            egui::TextEdit::singleline(&mut self.auto_code)
                                .hint_text("ABCD-2345")
                                .desired_width(f32::INFINITY),
                        );
                        let submit_with_enter = code_response.has_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        let busy = self.pairing_in_progress.is_some();
                        let label = if busy {
                            "Pairing..."
                        } else {
                            "Pair with this device"
                        };
                        let clicked = ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(
                                    RichText::new(label).strong().color(Color32::WHITE),
                                )
                                .fill(ACCENT_DARK)
                                .min_size(Vec2::new(ui.available_width(), 32.0)),
                            )
                            .clicked();
                        if clicked || (submit_with_enter && !busy) {
                            self.spawn_pair(peer.addr, self.auto_code.clone());
                        }
                        if self.pairing_in_progress == Some(peer.addr) {
                            ui.label(
                                RichText::new(
                                    "Waiting for approval on the tablet. Keep Shadow Transfer open there and tap Approve.",
                                )
                                .small()
                                .color(YELLOW),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "Approve the pairing request on the tablet when it appears.",
                                )
                                .small()
                                .color(MUTED),
                            );
                        }
                    }
                } else if !peers.is_empty() {
                    ui.label(
                        RichText::new("Select a discovered device to pair with it.")
                            .small()
                            .color(MUTED),
                    );
                }
            });
    }

    fn manual_connection_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(Rounding::same(10.0))
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("MANUAL CONNECTION")
                        .size(11.0)
                        .strong()
                        .color(MUTED),
                );
                ui.label(
                    RichText::new(
                        "Separate from auto discovery. Use when the tablet is not listed.",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Tablet IP address").small().color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut self.manual_host)
                        .hint_text("192.168.1.42")
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Port").small().color(MUTED));
                    let mut port_text = self.manual_port.clone();
                    let port_response = ui.add(
                        egui::TextEdit::singleline(&mut port_text)
                            .hint_text("47831")
                            .desired_width(72.0),
                    );
                    if port_response.changed() {
                        self.manual_port = port_text
                            .chars()
                            .filter(|c| c.is_ascii_digit())
                            .take(5)
                            .collect();
                    }
                    ui.label(
                        RichText::new("The code goes in the pairing-code field below.")
                            .small()
                            .color(MUTED),
                    );
                });
                ui.label(RichText::new("Pairing code").small().color(MUTED));
                let code_response = ui.add(
                    egui::TextEdit::singleline(&mut self.manual_code)
                        .hint_text("ABCD-2345")
                        .desired_width(f32::INFINITY),
                );
                let submit_with_enter = code_response.has_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                let busy = self.pairing_in_progress.is_some();
                let clicked = ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(
                            RichText::new(if busy { "Pairing..." } else { "Connect & pair" })
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT_DARK)
                        .min_size(Vec2::new(ui.available_width(), 32.0)),
                    )
                    .clicked();
                if clicked || (submit_with_enter && !busy) {
                    self.connect_manual();
                }
                if let Some(peer) = self.selected_peer().filter(|peer| peer.manual) {
                    if peer_trusted(&self.trust, &peer) {
                        ui.label(
                            RichText::new(format!("Paired with {}. Ready to send.", peer.name))
                                .small()
                                .color(GREEN),
                        );
                    } else {
                        ui.label(
                            RichText::new("Not paired yet. Enter the code shown on the tablet.")
                                .small()
                                .color(YELLOW),
                        );
                    }
                }
                if let Some(addr) = self.pairing_in_progress {
                    ui.label(
                        RichText::new(format!("Waiting for approval on the tablet for {addr}..."))
                            .small()
                            .color(YELLOW),
                    );
                }
            });
    }

    fn files_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("FILES TO SEND")
                    .size(11.0)
                    .strong()
                    .color(MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Folder").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.add_paths(vec![folder]);
                    }
                }
                if ui.button("Files").clicked() {
                    if let Some(files) = rfd::FileDialog::new().pick_files() {
                        self.add_paths(files);
                    }
                }
            });
        });
        if self.sources_loading {
            ui.label(RichText::new("Reading selection…").small().color(MUTED));
        }
        if self.sources.is_empty() {
            ui.label(
                RichText::new("Drop files or folders anywhere, or choose them above.")
                    .small()
                    .color(MUTED),
            );
        }
        let mut remove: Option<usize> = None;
        egui::ScrollArea::vertical()
            .max_height(170.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (index, source) in self.sources.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&source.rel_path).small().color(TEXT))
                            .on_hover_text(source.path.display().to_string());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                remove = Some(index);
                            }
                            ui.label(
                                RichText::new(shadow_transfer::util::format_bytes(source.size))
                                    .small()
                                    .color(MUTED),
                            );
                        });
                    });
                }
            });
        if let Some(index) = remove {
            self.sources.remove(index);
            self.total_bytes = self.sources.iter().map(|s| s.size).sum();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.sources.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                self.sources.clear();
                self.total_bytes = 0;
            }
            ui.label(
                RichText::new(format!(
                    "{} file(s) · {}",
                    self.sources.len(),
                    shadow_transfer::util::format_bytes(self.total_bytes)
                ))
                .small()
                .color(MUTED),
            );
        });
        ui.add_space(6.0);
        let selected = self.selected_peer();
        if let Some(peer) = &selected {
            let trusted = peer_trusted(&self.trust, peer);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Send to: {}", peer.name))
                        .small()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new(if trusted { "paired" } else { "not paired" })
                        .small()
                        .color(if trusted { GREEN } else { YELLOW }),
                );
            });
        } else {
            ui.label(RichText::new("No device selected.").small().color(YELLOW));
        }
        if ui
            .add(
                egui::Button::new(RichText::new("Send").strong().color(Color32::WHITE))
                    .fill(ACCENT_DARK)
                    .min_size(Vec2::new(ui.available_width(), 34.0)),
            )
            .clicked()
        {
            self.open_send_dialog();
        }
        let hint = if self.sources.is_empty() {
            "Add files or a folder first."
        } else if selected.is_none() {
            "Select or pair a device above."
        } else if selected
            .as_ref()
            .map(|peer| peer_trusted(&self.trust, peer))
            .unwrap_or(false)
        {
            "Ready. Press Send; approve the incoming transfer on the tablet."
        } else {
            "Pair the selected device first."
        };
        ui.label(RichText::new(hint).small().color(MUTED));
    }

    fn receive_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("RECEIVING").size(11.0).strong().color(MUTED));
        if self.receiver_starting {
            ui.label(RichText::new("Starting receiver…").color(YELLOW));
        }
        if let Some(addr) = self.receiver_addr {
            ui.label(
                RichText::new(format!("Listening on port {}", addr.port()))
                    .small()
                    .color(GREEN),
            );
        }
        if !self.pairing_code.is_empty() {
            let remaining = self
                .pairing_expires_at
                .map(|at| at.saturating_duration_since(Instant::now()).as_secs())
                .unwrap_or(self.pairing_expires as u64);
            egui::Frame::none()
                .fill(CARD)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .rounding(Rounding::same(8.0))
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("PAIRING CODE")
                            .size(10.0)
                            .strong()
                            .color(MUTED),
                    );
                    ui.label(
                        RichText::new(&self.pairing_code)
                            .monospace()
                            .size(28.0)
                            .strong()
                            .color(ACCENT),
                    );
                    ui.label(
                        RichText::new(if remaining > 0 {
                            format!("Expires in {remaining}s · enter on the sending device")
                        } else {
                            "Code expired · create a new code".to_string()
                        })
                        .small()
                        .color(if remaining > 0 { MUTED } else { YELLOW }),
                    );
                });
            ui.add_space(4.0);
            if ui.button("New code").clicked() {
                if let Some(handle) = &self.receiver {
                    self.pairing_code = handle.regenerate_pairing_code();
                }
            }
        }
        ui.label(
            RichText::new(format!(
                "Saving to {}",
                self.settings.download_dir.display()
            ))
            .small()
            .color(MUTED),
        );
        ui.label(
            RichText::new(
                "If the other device cannot find this PC, allow Shadow Transfer in Windows Firewall for private networks.",
            )
            .small()
            .color(MUTED),
        );
    }

    fn central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(22.0, 18.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.tab == Tab::Transfers, "Transfers")
                        .clicked()
                    {
                        self.tab = Tab::Transfers;
                    }
                    if ui
                        .selectable_label(self.tab == Tab::History, "History")
                        .clicked()
                    {
                        self.tab = Tab::History;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let active = self.cards.iter().filter(|c| c.is_active()).count();
                        if active > 0 {
                            ui.label(
                                RichText::new(format!("{active} active"))
                                    .small()
                                    .color(ACCENT),
                            );
                        }
                    });
                });
                ui.add_space(6.0);
                self.toast_bar(ui);
                ui.separator();
                ui.add_space(8.0);
                match self.tab {
                    Tab::Transfers => self.transfers_view(ui),
                    Tab::History => self.history_view(ui),
                }
            });
    }

    fn toast_bar(&mut self, ui: &mut egui::Ui) {
        let Some(toast) = self.toast.as_ref() else {
            return;
        };
        if toast.at.elapsed() > Duration::from_secs(10) {
            self.toast = None;
            return;
        }
        let color = if toast.error { RED } else { GREEN };
        let text = toast.text.clone();
        egui::Frame::none()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, color))
            .rounding(Rounding::same(8.0))
            .inner_margin(egui::Margin::symmetric(12.0, 8.0))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(text).small().color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Dismiss").clicked() {
                            self.toast = None;
                        }
                    });
                });
            });
        ui.add_space(6.0);
    }

    fn transfers_view(&mut self, ui: &mut egui::Ui) {
        if self.cards.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No transfers yet").size(16.0).color(MUTED));
                ui.label(
                    RichText::new("Select a device and files, then press Send.")
                        .small()
                        .color(MUTED),
                );
            });
            return;
        }
        let mut cancel_send: Option<String> = None;
        let mut cancel_receive: Option<String> = None;
        let mut retry: Option<usize> = None;
        let mut dismiss: Option<usize> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, card) in self.cards.iter().enumerate() {
                    egui::Frame::none()
                        .fill(CARD)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .rounding(Rounding::same(10.0))
                        .inner_margin(egui::Margin::same(14.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let direction = match card.direction {
                                    Direction::Send => "SEND",
                                    Direction::Receive => "RECEIVE",
                                };
                                ui.label(RichText::new(direction).size(10.0).strong().color(
                                    if card.direction == Direction::Send {
                                        ACCENT
                                    } else {
                                        GREEN
                                    },
                                ));
                                ui.label(RichText::new(&card.peer).small().color(MUTED));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let (label, color) = phase_label(card.phase);
                                    ui.label(RichText::new(label).small().strong().color(color));
                                });
                            });
                            let title = if card.file_name.is_empty() {
                                format!("{} file(s)", card.file_count)
                            } else {
                                format!(
                                    "{}  ({}/{})",
                                    card.file_name,
                                    card.file_index + 1,
                                    card.file_count
                                )
                            };
                            ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
                            ui.add_space(4.0);
                            ui.add(
                                egui::ProgressBar::new(card.percent())
                                    .fill(ACCENT)
                                    .animate(false)
                                    .show_percentage(),
                            );
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{} / {}",
                                        shadow_transfer::util::format_bytes(card.total_done),
                                        shadow_transfer::util::format_bytes(card.total_bytes)
                                    ))
                                    .small()
                                    .color(MUTED),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "· {}",
                                        shadow_transfer::util::format_speed(card.speed_bps)
                                    ))
                                    .small()
                                    .color(MUTED),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "avg {}",
                                        shadow_transfer::util::format_speed(card.avg_bps)
                                    ))
                                    .small()
                                    .color(MUTED),
                                );
                                if card.is_active() {
                                    ui.label(
                                        RichText::new(format!(
                                            "· ETA {}",
                                            shadow_transfer::util::format_eta(card.eta_secs)
                                        ))
                                        .small()
                                        .color(MUTED),
                                    );
                                }
                            });
                            if let Some(detail) = &card.detail {
                                ui.label(RichText::new(detail).small().color(
                                    if matches!(card.phase, TransferPhase::Completed) {
                                        GREEN
                                    } else if matches!(card.phase, TransferPhase::Failed) {
                                        RED
                                    } else {
                                        MUTED
                                    },
                                ));
                            } else if card.direction == Direction::Send
                                && matches!(card.phase, TransferPhase::Pairing)
                            {
                                ui.label(
                                    RichText::new(
                                        "Waiting for approval on the tablet. Open Shadow Transfer there and tap Approve.",
                                    )
                                    .small()
                                    .color(YELLOW),
                                );
                            }
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if card.is_active() && ui.button("Cancel").clicked() {
                                    match card.direction {
                                        Direction::Send => cancel_send = Some(card.id.clone()),
                                        Direction::Receive => {
                                            cancel_receive = Some(card.id.clone())
                                        }
                                    }
                                }
                                if !card.is_active()
                                    && card.job.is_some()
                                    && ui.button("Retry").clicked()
                                {
                                    retry = Some(index);
                                }
                                if !card.is_active() && ui.button("Dismiss").clicked() {
                                    dismiss = Some(index);
                                }
                            });
                        });
                    ui.add_space(10.0);
                }
            });
        if let Some(id) = cancel_send {
            if let Some(card) = self.card_mut(&id) {
                if let Some(token) = &card.cancel {
                    token.cancel();
                    card.detail = Some("Cancelling…".into());
                }
            }
        }
        if let Some(id) = cancel_receive {
            if let Some(handle) = &self.receiver {
                handle.cancel_transfer(&id);
            }
        }
        if let Some(index) = retry {
            self.retry_card(index);
        }
        if let Some(index) = dismiss {
            self.cards.remove(index);
        }
    }

    fn history_view(&mut self, ui: &mut egui::Ui) {
        if self.history.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("History is empty").color(MUTED));
            });
            return;
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} recent transfers", self.history.len()))
                    .small()
                    .color(MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Clear history").clicked() {
                    self.history.clear();
                    if let Err(err) =
                        storage::save_history(&self.data_dir.join("history.json"), &self.history)
                    {
                        self.set_toast(format!("Could not save history: {err}"), true);
                    }
                }
            });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("history-grid")
                    .num_columns(5)
                    .spacing(Vec2::new(18.0, 8.0))
                    .striped(true)
                    .show(ui, |ui| {
                        for heading in ["When", "Direction", "Peer", "Size", "Status"] {
                            ui.label(RichText::new(heading).size(10.0).strong().color(MUTED));
                        }
                        ui.end_row();
                        for entry in &self.history {
                            ui.label(
                                RichText::new(storage::relative_time(entry.at))
                                    .small()
                                    .color(MUTED),
                            );
                            ui.label(RichText::new(&entry.direction).small().color(TEXT));
                            ui.label(
                                RichText::new(format!("{} · {} file(s)", entry.peer, entry.files))
                                    .small()
                                    .color(TEXT),
                            );
                            ui.label(
                                RichText::new(shadow_transfer::util::format_bytes(entry.bytes))
                                    .small()
                                    .color(MUTED),
                            );
                            let color = match entry.status.as_str() {
                                "Verified" => GREEN,
                                "Failed" => RED,
                                "Cancelled" => YELLOW,
                                _ => MUTED,
                            };
                            ui.label(RichText::new(&entry.status).small().color(color))
                                .on_hover_text(&entry.detail);
                            ui.end_row();
                        }
                    });
            });
    }

    fn approval_window(&mut self, ctx: &egui::Context) {
        let Some(approval) = self.approvals.front().cloned() else {
            return;
        };
        let mut decision: Option<bool> = None;
        egui::Window::new("Approval required")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(approval.kind.label())
                        .size(16.0)
                        .strong()
                        .color(TEXT),
                );
                ui.add_space(4.0);
                match &approval.kind {
                    ApprovalKind::Pairing {
                        device_name,
                        fingerprint,
                    } => {
                        ui.label(
                            RichText::new(format!(
                                "{device_name} wants to pair with this device."
                            ))
                            .color(TEXT),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Only approve if you entered this device's code and you recognise it.")
                                .small()
                                .color(MUTED),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(fingerprint)
                                .monospace()
                                .small()
                                .color(MUTED),
                        );
                    }
                    ApprovalKind::Transfer {
                        sender_name,
                        file_count,
                        total_bytes,
                        preview,
                        ..
                    } => {
                        ui.label(
                            RichText::new(format!(
                                "{sender_name} wants to send {file_count} file(s), {}.",
                                shadow_transfer::util::format_bytes(*total_bytes)
                            ))
                            .color(TEXT),
                        );
                        ui.add_space(4.0);
                        for name in preview.iter().take(8) {
                            ui.label(RichText::new(format!("• {name}")).small().color(MUTED));
                        }
                        if *file_count > preview.len() {
                            ui.label(
                                RichText::new(format!(
                                    "… and {} more",
                                    file_count - preview.len()
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Approve").strong().color(Color32::WHITE))
                                .fill(ACCENT_DARK),
                        )
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    if ui.button("Decline").clicked() {
                        decision = Some(false);
                    }
                    if self.approvals.len() > 1 {
                        ui.label(
                            RichText::new(format!(
                                "{} more request(s) waiting",
                                self.approvals.len() - 1
                            ))
                            .small()
                            .color(MUTED),
                        );
                    }
                });
            });
        if let Some(accepted) = decision {
            if let Some(handle) = &self.receiver {
                let _ = handle.respond(&approval.id, accepted);
            }
            self.approvals.pop_front();
        }
    }

    fn send_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.send_dialog.take() else {
            return;
        };
        let mut action: Option<bool> = None;
        let total = self.total_bytes;
        let files = self.sources.len();
        egui::Window::new("Confirm send")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("To {}", dialog.peer.name))
                        .size(16.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(
                    RichText::new(format!(
                        "{files} file(s) · {} · {}",
                        shadow_transfer::util::format_bytes(total),
                        dialog.peer.addr
                    ))
                    .small()
                    .color(MUTED),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("This device is paired and trusted.")
                        .small()
                        .color(GREEN),
                );
                ui.label(
                    RichText::new(
                        "After pressing Send, tap Approve on the tablet for the incoming transfer.",
                    )
                    .small()
                    .color(YELLOW),
                );
                ui.add_space(10.0);
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    action = Some(true);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Send").strong().color(Color32::WHITE))
                                .fill(ACCENT_DARK),
                        )
                        .clicked()
                    {
                        action = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(false);
                    }
                });
            });
        match action {
            Some(true) => {
                self.begin_send(dialog.peer.clone(), String::new());
            }
            Some(false) => {}
            None => self.send_dialog = Some(dialog),
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let Some(mut draft) = self.settings_draft.take() else {
            return;
        };
        let mut save = false;
        let mut close = false;
        let mut revoke: Option<[u8; 32]> = None;
        let mut change_folder = false;
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("DEVICE").size(10.0).strong().color(MUTED));
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut draft.device_name).desired_width(260.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Receive port");
                    ui.add(egui::DragValue::new(&mut draft.receive_port).range(1024..=65535));
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("DOWNLOAD FOLDER")
                        .size(10.0)
                        .strong()
                        .color(MUTED),
                );
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(draft.download_dir.display().to_string())
                            .small()
                            .color(TEXT),
                    );
                    if ui.button("Change").clicked() {
                        change_folder = true;
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new("IDENTITY").size(10.0).strong().color(MUTED));
                ui.label(
                    RichText::new(self.identity.fingerprint_hex())
                        .monospace()
                        .small()
                        .color(MUTED),
                );
                ui.label(
                    RichText::new(format!("Data: {}", self.data_dir.display()))
                        .small()
                        .color(MUTED),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("TRUSTED DEVICES")
                        .size(10.0)
                        .strong()
                        .color(MUTED),
                );
                let trusted: Vec<TrustedDevice> = self.trust.list().unwrap_or_default();
                if trusted.is_empty() {
                    ui.label(RichText::new("No paired devices.").small().color(MUTED));
                }
                for device in &trusted {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&device.name).small().color(TEXT));
                        ui.label(
                            RichText::new(shadow_transfer::util::short_fingerprint(
                                &device.fingerprint,
                            ))
                            .monospace()
                            .small()
                            .color(MUTED),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("Revoke").clicked() {
                                if let Ok(bytes) = hex::decode(&device.fingerprint) {
                                    if let Ok(fingerprint) = bytes.try_into() {
                                        revoke = Some(fingerprint);
                                    }
                                }
                            }
                        });
                    });
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Save").strong().color(Color32::WHITE))
                                .fill(ACCENT_DARK),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if change_folder {
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                draft.download_dir = folder;
            }
        }
        if let Some(fingerprint) = revoke {
            match self.trust.revoke(&fingerprint) {
                Ok(_) => {
                    self.set_toast("Device trust revoked", false);
                }
                Err(err) => self.set_toast(format!("Cannot update trust store: {err}"), true),
            }
        }
        if save {
            self.settings_draft = Some(draft);
            self.save_settings();
        } else if close {
            self.settings_open = false;
            self.settings_draft = None;
        } else {
            self.settings_draft = Some(draft);
        }
    }

    fn paint_drop_overlay(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop-overlay"),
        ));
        let rect = ctx.screen_rect();
        painter.rect_filled(
            rect,
            Rounding::same(0.0),
            Color32::from_rgba_unmultiplied(8, 9, 11, 220),
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop files or folders to add them to the send queue",
            egui::FontId::proportional(20.0),
            TEXT,
        );
    }
}

fn peer_is_online(peer: &PeerView) -> bool {
    peer.manual
        || peer
            .last_seen
            .map(|seen| seen.elapsed() <= Duration::from_secs(12))
            .unwrap_or(false)
}

fn peer_trusted(trust: &TrustStore, peer: &PeerView) -> bool {
    peer.fingerprint
        .as_deref()
        .and_then(parse_fingerprint)
        .map(|fingerprint| trust.is_trusted(&fingerprint))
        .unwrap_or(false)
}

fn parse_fingerprint(value: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(value).ok()?;
    bytes.try_into().ok()
}

fn phase_label(phase: TransferPhase) -> (&'static str, Color32) {
    match phase {
        TransferPhase::Connecting => ("Connecting", MUTED),
        TransferPhase::Pairing => ("Pairing", YELLOW),
        TransferPhase::Transferring => ("Transferring", ACCENT),
        TransferPhase::Verifying => ("Verifying", YELLOW),
        TransferPhase::Completed => ("Verified", GREEN),
        TransferPhase::Failed => ("Failed", RED),
        TransferPhase::Cancelled => ("Cancelled", YELLOW),
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = Color32::from_rgb(6, 7, 9);
    visuals.faint_bg_color = Color32::from_rgb(22, 24, 30);
    visuals.selection.bg_fill = ACCENT_DARK;
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(32, 34, 42);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(32, 34, 42);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.active.bg_fill = ACCENT_DARK;
    visuals.widgets.active.weak_bg_fill = ACCENT_DARK;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.noninteractive,
    ] {
        widget.rounding = Rounding::same(7.0);
    }
    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(9.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    ctx.set_style(style);
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        ("msyh", "C:\\Windows\\Fonts\\msyh.ttc"),
        ("simsun", "C:\\Windows\\Fonts\\simsun.ttc"),
        ("segoe", "C:\\Windows\\Fonts\\segoeui.ttf"),
    ];
    for (name, path) in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        fonts
            .font_data
            .insert(name.to_string(), egui::FontData::from_owned(bytes));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push(name.to_string());
        }
    }
    ctx.set_fonts(fonts);
}

impl eframe::App for ShadowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.events_rx.try_recv() {
            self.handle_app_event(ctx, event);
        }
        if !self.discovery_running && Instant::now() >= self.next_discovery_at {
            let focused = ctx.input(|input| input.viewport().focused.unwrap_or(true));
            let minimized = ctx.input(|input| input.viewport().minimized.unwrap_or(false));
            if focused && !minimized {
                self.start_discovery();
            }
        }
        self.handle_drops(ctx);
        if !self.demo_autosend_started {
            if let Some((addr, code)) = self.demo_autosend.clone() {
                if !self.sources.is_empty() {
                    self.demo_autosend_started = true;
                    let peer = PeerView {
                        name: "Demo Receiver".into(),
                        addr,
                        fingerprint: None,
                        manual: true,
                        last_seen: None,
                    };
                    self.selected_addr = Some(addr);
                    self.peers.push(peer.clone());
                    self.begin_send(peer, code);
                }
            }
        }
        self.header(ctx);
        self.side_panel(ctx);
        self.central_panel(ctx);
        self.approval_window(ctx);
        self.send_window(ctx);
        self.settings_window(ctx);
        self.paint_drop_overlay(ctx);
        if self.ongoing() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            // Idle repaint drives the automatic discovery cadence without a
            // busy loop; the focused-window check still gates network use.
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }
}

pub enum RootApp {
    Ready(Box<ShadowApp>),
    Fatal(FatalApp),
}

pub struct FatalApp {
    message: String,
}

impl RootApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        install_fonts(&cc.egui_ctx);
        match ShadowApp::try_new(cc) {
            Ok(app) => Self::Ready(Box::new(app)),
            Err(message) => Self::Fatal(FatalApp { message }),
        }
    }
}

impl eframe::App for RootApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match self {
            Self::Ready(app) => eframe::App::update(app.as_mut(), ctx, frame),
            Self::Fatal(fatal) => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(120.0);
                        ui.label(
                            RichText::new("Shadow Transfer could not start")
                                .size(22.0)
                                .color(RED),
                        );
                        ui.label(RichText::new(&fatal.message).color(TEXT));
                        ui.label(
                            RichText::new(
                                "Check permissions for %LOCALAPPDATA%\\ShadowTransfer and try again.",
                            )
                            .small()
                            .color(MUTED),
                        );
                    });
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(last_seen: Option<Instant>, manual: bool) -> PeerView {
        PeerView {
            name: "Test".into(),
            addr: "127.0.0.1:47831".parse().unwrap(),
            fingerprint: None,
            manual,
            last_seen,
        }
    }

    #[test]
    fn presence_uses_a_twelve_second_window() {
        assert!(peer_is_online(&peer(Some(Instant::now()), false)));
        assert!(peer_is_online(&peer(
            Some(Instant::now() - Duration::from_secs(5)),
            false
        )));
        assert!(!peer_is_online(&peer(
            Some(Instant::now() - Duration::from_secs(20)),
            false
        )));
        assert!(peer_is_online(&peer(None, true)));
    }
}
