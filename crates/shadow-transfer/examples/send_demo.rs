//! Headless sender used for exercising the desktop UI and for manual testing.
//!
//! Usage: send_demo <host> <port> <pairing-code> <path>

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use shadow_crypto::{Identity, TrustStore};
use shadow_transfer::collect_sources;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: send_demo <host> <port> <pairing-code> <path>");
        std::process::exit(2);
    }
    let addr = format!("{}:{}", args[1], args[2])
        .parse()
        .expect("invalid address");
    let code = args[3].clone();
    let path = PathBuf::from(&args[4]);

    let data_dir = std::env::temp_dir().join("shadow-transfer-demo-sender");
    let identity = Arc::new(Identity::load_or_create(&data_dir).expect("identity"));
    let trust = Arc::new(TrustStore::load(data_dir.join("trusted.json")).expect("trust"));
    let mut config = shadow_transfer::SenderConfig::new(addr, identity, trust, "Demo Sender");
    config.pairing_code = Some(code);
    config.approval_timeout = Duration::from_secs(300);

    let sources = collect_sources(vec![path]).await.expect("sources");
    let total = sources.iter().map(|s| s.size).sum::<u64>();
    println!("sending {} file(s), {total} bytes", sources.len());

    let (progress_tx, mut progress_rx) =
        tokio::sync::mpsc::unbounded_channel::<shadow_transfer::ProgressEvent>();
    tokio::spawn(async move {
        while let Some(event) = progress_rx.recv().await {
            println!(
                "{:?} {:.1}% {} {:.1} MB/s",
                event.phase,
                event.percent() * 100.0,
                event.file_name,
                event.speed_bps / 1_000_000.0
            );
        }
    });

    match shadow_transfer::send_sources(
        &config,
        None,
        sources,
        progress_tx,
        CancellationToken::new(),
    )
    .await
    {
        Ok(summary) => {
            println!(
                "OK files={} sent={} reused={} avg={:.1} MB/s",
                summary.files.len(),
                summary.bytes_sent,
                summary.bytes_reused,
                summary.avg_bps / 1_000_000.0
            );
        }
        Err(err) => {
            eprintln!("ERR {err}");
            std::process::exit(1);
        }
    }
}
