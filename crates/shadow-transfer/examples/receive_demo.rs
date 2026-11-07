//! Headless receiver used for exercising the desktop UI and manual tests.
//!
//! Usage: receive_demo <port> <destination> [device-name]

use std::path::PathBuf;
use std::sync::Arc;

use shadow_crypto::{Identity, TrustStore};
use shadow_transfer::{ReceiverConfig, ReceiverEvent};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().expect("port").parse().expect("valid port");
    let destination = PathBuf::from(args.next().expect("destination"));
    let name = args.next().unwrap_or_else(|| "Demo Receiver".to_string());

    let data_dir = std::env::temp_dir().join("shadow-transfer-demo-receiver");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let identity = Arc::new(Identity::load_or_create(&data_dir).expect("identity"));
    let trust = Arc::new(TrustStore::load(data_dir.join("trusted.json")).expect("trust"));

    let config = ReceiverConfig::new(
        format!("127.0.0.1:{port}").parse().expect("bind address"),
        destination.clone(),
        name,
    );
    let (handle, mut events) = shadow_transfer::start(config, identity, trust)
        .await
        .expect("receiver start");
    println!("PAIRING_CODE={}", handle.pairing_code());
    println!("DESTINATION={}", destination.display());
    use std::io::Write;
    let _ = std::io::stdout().flush();

    while let Some(event) = events.recv().await {
        match event {
            ReceiverEvent::ApprovalNeeded(approval) => {
                println!(
                    "APPROVAL {} {:?}",
                    approval.kind.label(),
                    approval.kind.summary()
                );
                let _ = handle.respond(&approval.id, true);
            }
            ReceiverEvent::FileReceived {
                file_name, bytes, ..
            } => println!("RECEIVED {file_name} {bytes}"),
            ReceiverEvent::TransferFinished {
                files,
                bytes,
                duration_secs,
                ..
            } => println!("FINISHED files={files} bytes={bytes} duration={duration_secs:.2}s"),
            ReceiverEvent::TransferFailed { error, .. } => println!("FAILED {error}"),
            ReceiverEvent::PairingCode { code, .. } => println!("PAIRING_CODE={code}"),
            _ => {}
        }
        let _ = std::io::stdout().flush();
    }
}
