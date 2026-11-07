use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use shadow_crypto::{Identity, TrustStore};
use shadow_transfer::{ApprovalKind, ReceiverConfig, ReceiverEvent, ReceiverHandle};
use tempfile::TempDir;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TestEvent {
    Approval(ApprovalKind),
    FileReceived { name: String, destination: PathBuf },
    Finished { files: usize, bytes: u64 },
    Failed(String),
}

pub struct TestReceiver {
    pub handle: Arc<ReceiverHandle>,
    pub events: UnboundedReceiver<TestEvent>,
    pub auto_approve: Arc<AtomicBool>,
}

pub struct Peer {
    pub dir: TempDir,
    pub identity: Arc<Identity>,
    pub trust: Arc<TrustStore>,
}

impl Peer {
    pub fn new(parent: &Path, name: &str) -> Self {
        let dir = tempfile::tempdir_in(parent).unwrap();
        let identity = Arc::new(Identity::load_or_create(dir.path()).unwrap());
        let trust = Arc::new(TrustStore::load(dir.path().join("trusted.json")).unwrap());
        let _ = name;
        Self {
            dir,
            identity,
            trust,
        }
    }
}

pub async fn start_receiver(
    parent: &Path,
    name: &str,
    auto_approve: bool,
) -> (TestReceiver, Peer, PathBuf) {
    let peer = Peer::new(parent, name);
    let destination = peer.dir.path().join("received");
    std::fs::create_dir_all(&destination).unwrap();
    let config = ReceiverConfig::new("127.0.0.1:0".parse().unwrap(), destination.clone(), name);
    let (handle, mut raw_events) =
        shadow_transfer::start(config, peer.identity.clone(), peer.trust.clone())
            .await
            .unwrap();
    let handle = Arc::new(handle);
    let (tx, rx) = unbounded_channel();
    let auto = Arc::new(AtomicBool::new(auto_approve));
    let task_handle = Arc::clone(&handle);
    let task_auto = Arc::clone(&auto);
    tokio::spawn(async move {
        while let Some(event) = raw_events.recv().await {
            match event {
                ReceiverEvent::ApprovalNeeded(approval) => {
                    let accepted = task_auto.load(Ordering::SeqCst);
                    let _ = task_handle.respond(&approval.id, accepted);
                    let _ = tx.send(TestEvent::Approval(approval.kind));
                }
                ReceiverEvent::FileReceived {
                    file_name,
                    destination,
                    ..
                } => {
                    let _ = tx.send(TestEvent::FileReceived {
                        name: file_name,
                        destination,
                    });
                }
                ReceiverEvent::TransferFinished { files, bytes, .. } => {
                    let _ = tx.send(TestEvent::Finished { files, bytes });
                }
                ReceiverEvent::TransferFailed { error, .. } => {
                    let _ = tx.send(TestEvent::Failed(error));
                }
                _ => {}
            }
        }
    });
    (
        TestReceiver {
            handle,
            events: rx,
            auto_approve: auto,
        },
        peer,
        destination,
    )
}

pub fn write_pattern(path: &Path, size: u64) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut data = Vec::with_capacity(size.min(8 * 1024 * 1024) as usize);
    let mut remaining = size;
    while remaining > 0 {
        let take = remaining.min(1024 * 1024) as usize;
        data.clear();
        for _ in 0..take {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            data.push(state as u8);
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(&data).unwrap();
        remaining -= take as u64;
    }
}

pub async fn next_event(
    events: &mut UnboundedReceiver<TestEvent>,
    timeout: std::time::Duration,
) -> Option<TestEvent> {
    tokio::time::timeout(timeout, events.recv())
        .await
        .ok()
        .flatten()
}

pub async fn wait_for(
    events: &mut UnboundedReceiver<TestEvent>,
    timeout: std::time::Duration,
    predicate: impl Fn(&TestEvent) -> bool,
) -> Option<TestEvent> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match next_event(events, remaining).await {
            Some(event) => {
                if predicate(&event) {
                    return Some(event);
                }
            }
            None => return None,
        }
    }
}
