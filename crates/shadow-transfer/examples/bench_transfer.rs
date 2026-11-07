//! End-to-end loopback benchmark for the real transfer engine.
//!
//! Measures complete transfer time including BLAKE3 verification by the
//! receiver. Both peers run on this machine over 127.0.0.1, so results are an
//! upper bound, not a prediction of Wi-Fi throughput.
//!
//! Usage: bench_transfer [sizes_mb] [chunk_kb]
//! Default sizes: 1,10,50,100,500,1024 MB.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use shadow_crypto::{Identity, TrustStore};
use shadow_transfer::{collect_sources, ChunkPolicy, ReceiverConfig, ReceiverEvent, SenderConfig};
use tokio_util::sync::CancellationToken;

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn create_file(path: &Path, size: u64) {
    use std::io::Write;
    let mut file = std::fs::File::create(path).expect("create benchmark file");
    let block = vec![0xA5u8; 8 * 1024 * 1024];
    let mut remaining = size;
    while remaining > 0 {
        let take = remaining.min(block.len() as u64) as usize;
        file.write_all(&block[..take])
            .expect("write benchmark file");
        remaining -= take as u64;
    }
    file.sync_all().expect("sync benchmark file");
}

async fn one_transfer(sender: &SenderConfig, base: &Path, file: &Path, label: &str) -> f64 {
    let sources = collect_sources(vec![file.to_path_buf()])
        .await
        .expect("collect");
    let total = sources.iter().map(|s| s.size).sum::<u64>();
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let drain = tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
    let started = Instant::now();
    let summary =
        shadow_transfer::send_sources(sender, None, sources, progress_tx, CancellationToken::new())
            .await
            .expect("benchmark transfer");
    let elapsed = started.elapsed();
    let _ = drain.await;
    assert_eq!(summary.bytes_sent + summary.bytes_reused, total);
    let mbps = mib(total) / elapsed.as_secs_f64();
    println!(
        "{label:<28} {:>7.1} MiB  {:>8.1} ms  {:>8.1} MiB/s",
        mib(total),
        elapsed.as_secs_f64() * 1000.0,
        mbps
    );
    let _ = base;
    mbps
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sizes_mb: Vec<u64> = args
        .first()
        .map(|value| value.split(',').filter_map(|v| v.parse().ok()).collect())
        .unwrap_or_else(|| vec![1, 10, 50, 100, 500, 1024]);
    let chunk_kb: usize = args
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1024);

    let root = tempfile::tempdir().expect("bench temp dir");
    let sender_dir = root.path().join("sender");
    let receiver_dir = root.path().join("receiver");
    let destination = root.path().join("destination");
    let sender_identity = Arc::new(Identity::load_or_create(&sender_dir).expect("identity"));
    let sender_trust = Arc::new(TrustStore::load(sender_dir.join("trusted.json")).expect("trust"));
    let receiver_identity = Arc::new(Identity::load_or_create(&receiver_dir).expect("identity"));
    let receiver_trust =
        Arc::new(TrustStore::load(receiver_dir.join("trusted.json")).expect("trust"));

    let config = ReceiverConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        destination.clone(),
        "Benchmark Receiver",
    );
    let (handle, mut events) = shadow_transfer::start(config, receiver_identity, receiver_trust)
        .await
        .expect("receiver");
    let addr = handle.local_addr();
    let code = handle.pairing_code();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let ReceiverEvent::ApprovalNeeded(approval) = event {
                let _ = handle.respond(&approval.id, true);
            }
        }
    });

    let mut sender = SenderConfig::new(addr, sender_identity, sender_trust, "Benchmark Sender");
    sender.pairing_code = Some(code);

    println!("Shadow Transfer loopback benchmark (includes BLAKE3 verification)");
    println!("chunk sweep, 100 MiB file");
    let chunk_file = root.path().join("chunk-sweep.bin");
    create_file(&chunk_file, 100 * 1024 * 1024);
    for size_kb in [256usize, 512, 1024, 2048, 4096] {
        sender.chunk_policy = ChunkPolicy::Fixed(size_kb * 1024);
        let label = format!("chunk {size_kb} KiB");
        one_transfer(&sender, root.path(), &chunk_file, &label).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    sender.chunk_policy = ChunkPolicy::Adaptive;
    one_transfer(&sender, root.path(), &chunk_file, "chunk adaptive").await;

    println!();
    println!("file-size sweep, {chunk_kb} KiB chunks");
    sender.chunk_policy = ChunkPolicy::Fixed(chunk_kb * 1024);
    let mut total_mib = 0.0;
    let mut total_time = 0.0;
    for size in sizes_mb {
        let path: PathBuf = root.path().join(format!("size-{size}mb.bin"));
        create_file(&path, size * 1024 * 1024);
        let label = format!("{size} MiB");
        let started = Instant::now();
        one_transfer(&sender, root.path(), &path, &label).await;
        total_time += started.elapsed().as_secs_f64();
        total_mib += size as f64;
        let _ = std::fs::remove_file(&path);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!();
    println!(
        "aggregate: {:.1} MiB in {:.2} s = {:.1} MiB/s",
        total_mib,
        total_time,
        total_mib / total_time
    );
    println!();
    let raw_started = Instant::now();
    let raw_mib = raw_tcp_loopback(1024 * 1024 * 1024).await;
    let raw_secs = raw_started.elapsed().as_secs_f64();
    println!(
        "raw TCP loopback           {:>7.1} MiB  {:>8.1} ms  {:>8.1} MiB/s",
        raw_mib,
        raw_secs * 1000.0,
        raw_mib / raw_secs
    );
    println!("note: loopback only; Wi-Fi/Android results require the physical tablet");
}

/// Raw TCP throughput with no TLS, framing or hashing, for comparison.
async fn raw_tcp_loopback(total_bytes: u64) -> f64 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("raw listener");
    let addr = listener.local_addr().unwrap();
    let reader = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("raw accept");
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut read_total = 0u64;
        loop {
            let read = socket.read(&mut buffer).await.expect("raw read");
            if read == 0 {
                break;
            }
            read_total += read as u64;
        }
        read_total
    });
    let mut client = TcpStream::connect(addr).await.expect("raw connect");
    let block = vec![0u8; 1024 * 1024];
    let mut sent = 0u64;
    while sent < total_bytes {
        let take = (total_bytes - sent).min(block.len() as u64) as usize;
        client.write_all(&block[..take]).await.expect("raw write");
        sent += take as u64;
    }
    client.shutdown().await.ok();
    let received = reader.await.expect("raw reader");
    assert_eq!(received, total_bytes);
    mib(received)
}
