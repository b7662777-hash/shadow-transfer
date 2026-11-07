mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{next_event, start_receiver, wait_for, write_pattern, Peer, TestEvent};
use shadow_crypto::{unix_now, Identity};
use shadow_protocol::{
    read_message, write_frame, write_message, FileChunk, FileStart, FrameKind, Hello, HelloAck,
    Message, SessionReady, TransferRequest, PROTOCOL_VERSION,
};
use shadow_transfer::{
    collect_sources, pair_with, send_sources, ChunkPolicy, ProgressEvent, SenderConfig,
    TransferError, TransferPhase,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc::unbounded_channel;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

fn make_sender(
    peer: &Peer,
    addr: std::net::SocketAddr,
    receiver: &Peer,
    code: Option<String>,
) -> SenderConfig {
    let mut config = SenderConfig::new(
        addr,
        peer.identity.clone(),
        peer.trust.clone(),
        "Test Workstation",
    );
    config.pairing_code = code;
    let _ = receiver;
    config
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paired_transfer_delivers_verified_files() {
    let root = tempfile::tempdir().unwrap();
    let (mut receiver, receiver_peer, destination) =
        start_receiver(root.path(), "Galaxy Tab A7", true).await;
    let sender = Peer::new(root.path(), "sender");

    let source_root = sender.dir.path().join("My Folder");
    std::fs::create_dir_all(source_root.join("sub")).unwrap();
    std::fs::write(source_root.join("hello.txt"), b"hello shadow transfer").unwrap();
    std::fs::write(source_root.join("empty.bin"), b"").unwrap();
    std::fs::write(source_root.join("sub").join("naïve—文件.dat"), b"unicode!").unwrap();
    write_pattern(&source_root.join("big.bin"), 5 * 1024 * 1024);

    let sources = collect_sources(vec![source_root.clone()]).await.unwrap();
    let total = sources.iter().map(|s| s.size).sum::<u64>();

    let code = receiver.handle.pairing_code();
    let config = make_sender(
        &sender,
        receiver.handle.local_addr(),
        &receiver_peer,
        Some(code),
    );
    let (progress_tx, mut progress_rx) = unbounded_channel::<ProgressEvent>();
    let summary = send_sources(
        &config,
        None,
        sources.clone(),
        progress_tx,
        CancellationToken::new(),
    )
    .await
    .expect("transfer should succeed");

    assert_eq!(summary.files.len(), sources.len());
    assert_eq!(summary.bytes_sent, total);
    assert_eq!(summary.bytes_reused, 0);
    assert_eq!(
        summary.peer_fingerprint,
        receiver_peer.identity.fingerprint_hex()
    );

    let finished = wait_for(&mut receiver.events, Duration::from_secs(30), |event| {
        matches!(event, TestEvent::Finished { .. })
    })
    .await
    .expect("receiver should report completion");
    match finished {
        TestEvent::Finished { files, bytes } => {
            assert_eq!(files, sources.len());
            assert_eq!(bytes, total);
        }
        _ => unreachable!(),
    }

    for source in &sources {
        let relative = shadow_transfer::verify_wire_path(&source.rel_path).unwrap();
        let delivered = destination.join(&relative);
        assert!(delivered.is_file(), "missing {}", delivered.display());
        let source_bytes = std::fs::read(&source.path).unwrap();
        let delivered_bytes = std::fs::read(&delivered).unwrap();
        assert_eq!(
            source_bytes, delivered_bytes,
            "content mismatch for {}",
            source.rel_path
        );
    }

    assert!(sender
        .trust
        .is_trusted(&receiver_peer.identity.fingerprint()));
    assert!(receiver_peer
        .trust
        .is_trusted(&sender.identity.fingerprint()));

    let mut phases = Vec::new();
    while let Ok(event) = progress_rx.try_recv() {
        phases.push(event.phase);
    }
    assert!(phases.contains(&TransferPhase::Transferring));
    assert!(phases.contains(&TransferPhase::Completed));

    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_pairing_code_is_rejected_then_correct_code_works() {
    let root = tempfile::tempdir().unwrap();
    let (receiver, receiver_peer, _destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");
    let source = sender.dir.path().join("note.txt");
    std::fs::write(&source, b"hello").unwrap();
    let sources = collect_sources(vec![source]).await.unwrap();

    let good = receiver.handle.pairing_code();
    let first = good.chars().next().unwrap();
    let replacement = if first == 'A' { 'B' } else { 'A' };
    let mut wrong: String = replacement.to_string();
    wrong.push_str(&good[1..]);

    let mut config = make_sender(
        &sender,
        receiver.handle.local_addr(),
        &receiver_peer,
        Some(wrong),
    );
    let (tx, _rx) = unbounded_channel();
    let error = send_sources(&config, None, sources.clone(), tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransferError::PairingRejected(_)),
        "unexpected error: {error}"
    );
    assert!(!sender
        .trust
        .is_trusted(&receiver_peer.identity.fingerprint()));
    assert!(!receiver_peer
        .trust
        .is_trusted(&sender.identity.fingerprint()));

    config.pairing_code = Some(good);
    let (tx, _rx) = unbounded_channel();
    let summary = send_sources(&config, None, sources, tx, CancellationToken::new())
        .await
        .expect("correct code should pair");
    assert_eq!(summary.files.len(), 1);
    assert!(receiver_peer
        .trust
        .is_trusted(&sender.identity.fingerprint()));

    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn standalone_pairing_then_send_without_code() {
    let root = tempfile::tempdir().unwrap();
    let (receiver, receiver_peer, destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");
    let source = sender.dir.path().join("paired.txt");
    std::fs::write(&source, b"pair first, send later").unwrap();
    let sources = collect_sources(vec![source]).await.unwrap();

    let code = receiver.handle.pairing_code();
    let mut config = make_sender(
        &sender,
        receiver.handle.local_addr(),
        &receiver_peer,
        Some(code),
    );
    let pair = pair_with(&config)
        .await
        .expect("standalone pairing should succeed");
    assert_eq!(
        pair.peer_fingerprint,
        receiver_peer.identity.fingerprint_hex()
    );
    assert!(sender
        .trust
        .is_trusted(&receiver_peer.identity.fingerprint()));
    assert!(receiver_peer
        .trust
        .is_trusted(&sender.identity.fingerprint()));

    config.pairing_code = None;
    let (tx, _rx) = unbounded_channel();
    let summary = send_sources(&config, None, sources, tx, CancellationToken::new())
        .await
        .expect("sending should not require the code after pairing");
    assert_eq!(summary.files.len(), 1);
    let delivered = destination.join("paired.txt");
    assert_eq!(std::fs::read(delivered).unwrap(), b"pair first, send later");
    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn untrusted_sender_without_code_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let (receiver, receiver_peer, _destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");
    let source = sender.dir.path().join("note.txt");
    std::fs::write(&source, b"hello").unwrap();
    let sources = collect_sources(vec![source]).await.unwrap();

    let config = make_sender(&sender, receiver.handle.local_addr(), &receiver_peer, None);
    let (tx, _rx) = unbounded_channel();
    let error = send_sources(&config, None, sources, tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransferError::PairingRequired),
        "unexpected error: {error}"
    );
    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_transfer_resumes_from_verified_prefix() {
    let root = tempfile::tempdir().unwrap();
    let (mut receiver, receiver_peer, destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");

    let source = sender.dir.path().join("large.bin");
    write_pattern(&source, 64 * 1024 * 1024);
    let sources = collect_sources(vec![source.clone()]).await.unwrap();
    let total = sources[0].size;
    let transfer_id = "ab".repeat(16);

    let code = receiver.handle.pairing_code();
    let mut config = make_sender(
        &sender,
        receiver.handle.local_addr(),
        &receiver_peer,
        Some(code),
    );
    config.chunk_policy = ChunkPolicy::Fixed(256 * 1024);

    let (tx, mut rx) = unbounded_channel::<ProgressEvent>();
    let cancel = CancellationToken::new();
    let watcher_cancel = cancel.clone();
    let watcher = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if event.total_bytes_done > 1024 * 1024 {
                watcher_cancel.cancel();
                break;
            }
        }
    });
    let error = send_sources(
        &config,
        Some(transfer_id.clone()),
        sources.clone(),
        tx,
        cancel,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, TransferError::Cancelled),
        "unexpected: {error}"
    );
    watcher.abort();

    let failed = wait_for(&mut receiver.events, Duration::from_secs(20), |event| {
        matches!(event, TestEvent::Failed(_))
    })
    .await;
    assert!(failed.is_some(), "receiver should notice the interruption");

    let (tx, _rx) = unbounded_channel();
    let summary = send_sources(
        &config,
        Some(transfer_id),
        sources.clone(),
        tx,
        CancellationToken::new(),
    )
    .await
    .expect("resumed transfer should succeed");
    assert!(
        summary.files[0].resumed_from > 0,
        "expected a resumed prefix, got {}",
        summary.files[0].resumed_from
    );
    assert_eq!(summary.bytes_reused, summary.files[0].resumed_from);
    assert_eq!(summary.bytes_sent + summary.bytes_reused, total);

    let finished = wait_for(&mut receiver.events, Duration::from_secs(30), |event| {
        matches!(event, TestEvent::Finished { .. })
    })
    .await
    .expect("receiver should finish");
    match finished {
        TestEvent::Finished { bytes, .. } => assert_eq!(bytes, total),
        _ => unreachable!(),
    }

    let delivered = destination.join("large.bin");
    let delivered_bytes = std::fs::read(&delivered).unwrap();
    let source_bytes = std::fs::read(&source).unwrap();
    assert_eq!(delivered_bytes.len(), source_bytes.len());
    assert_eq!(delivered_bytes, source_bytes);
    assert_eq!(delivered_bytes.len() as u64, total);

    receiver.handle.shutdown().await;
}

async fn raw_trusted_session(
    addr: std::net::SocketAddr,
    identity: &Identity,
    receiver_fingerprint: [u8; 32],
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let tcp = TcpStream::connect(addr).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    let connector = TlsConnector::from(shadow_crypto::client_config(identity).unwrap());
    let server_name =
        shadow_crypto::rustls::pki_types::ServerName::try_from("shadow-transfer.invalid").unwrap();
    let mut stream = connector.connect(server_name, tcp).await.unwrap();
    let certs = stream.get_ref().1.peer_certificates().unwrap();
    assert_eq!(
        shadow_crypto::fingerprint(certs[0].as_ref()),
        receiver_fingerprint
    );
    write_message(
        &mut stream,
        &Message::Hello(Hello {
            protocol: PROTOCOL_VERSION,
            device_name: "raw-client".into(),
            nonce: "01".repeat(16),
        }),
    )
    .await
    .unwrap();
    let ack = read_message(&mut stream).await.unwrap();
    match ack {
        Message::HelloAck(HelloAck { client_trusted, .. }) => assert!(client_trusted),
        other => panic!("unexpected {:?}", other.name()),
    }
    let ready = read_message(&mut stream).await.unwrap();
    assert!(matches!(ready, Message::SessionReady(SessionReady { .. })));
    stream
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn receiver_rejects_path_traversal_from_raw_client() {
    let root = tempfile::tempdir().unwrap();
    let (receiver, receiver_peer, destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");
    receiver_peer
        .trust
        .trust("raw-client", &sender.identity.fingerprint(), unix_now())
        .unwrap();

    let mut stream = raw_trusted_session(
        receiver.handle.local_addr(),
        &sender.identity,
        receiver_peer.identity.fingerprint(),
    )
    .await;

    let malicious = format!(
        r#"{{"type":"TRANSFER_REQUEST","payload":{{"transfer_id":"{}","sender_name":"raw","files":[{{"file_id":0,"rel_path":"../evil.txt","size":4,"modified_ms":0}}],"total_bytes":4}}}}"#,
        "cd".repeat(16)
    );
    write_frame(&mut stream, FrameKind::Control, malicious.as_bytes())
        .await
        .unwrap();

    let response = tokio::time::timeout(Duration::from_secs(5), read_message(&mut stream)).await;
    match response {
        Ok(Ok(Message::Error(error))) => assert_eq!(error.code, 400),
        Ok(Ok(other)) => panic!("expected ERROR, got {}", other.name()),
        Ok(Err(_)) => {}
        Err(_) => panic!("receiver did not respond to malicious metadata"),
    }

    let escaped = destination.parent().unwrap().join("evil.txt");
    assert!(
        !escaped.exists(),
        "path traversal must not write outside destination"
    );
    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn receiver_rejects_corrupt_chunk_hash() {
    let root = tempfile::tempdir().unwrap();
    let (receiver, receiver_peer, destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");
    receiver_peer
        .trust
        .trust("raw-client", &sender.identity.fingerprint(), unix_now())
        .unwrap();

    let mut stream = raw_trusted_session(
        receiver.handle.local_addr(),
        &sender.identity,
        receiver_peer.identity.fingerprint(),
    )
    .await;

    let transfer_id = "ef".repeat(16);
    let request = TransferRequest {
        transfer_id: transfer_id.clone(),
        sender_name: "raw".into(),
        files: vec![shadow_protocol::FileMeta {
            file_id: 0,
            rel_path: "bad.bin".into(),
            size: 1024,
            modified_ms: 0,
        }],
        total_bytes: 1024,
    };
    write_message(&mut stream, &Message::TransferRequest(request))
        .await
        .unwrap();
    let accepted = read_message(&mut stream).await.unwrap();
    assert!(matches!(accepted, Message::TransferAccept(_)));

    write_message(
        &mut stream,
        &Message::FileStart(FileStart {
            file_id: 0,
            offset: 0,
        }),
    )
    .await
    .unwrap();
    let payload = vec![0xABu8; 1024];
    let wrong_hash = blake3::hash(&vec![0u8; 1024]).to_hex().to_string();
    write_message(
        &mut stream,
        &Message::FileChunk(FileChunk {
            file_id: 0,
            offset: 0,
            len: payload.len() as u32,
            hash: wrong_hash,
        }),
    )
    .await
    .unwrap();
    shadow_protocol::write_data(&mut stream, &payload)
        .await
        .unwrap();

    let response = tokio::time::timeout(Duration::from_secs(10), read_message(&mut stream)).await;
    match response {
        Ok(Ok(Message::Error(error))) => assert_eq!(error.code, 400),
        Ok(Err(_)) => {}
        Ok(Ok(other)) => panic!("expected ERROR, got {}", other.name()),
        Err(_) => panic!("receiver did not reject corrupt chunk"),
    }
    assert!(!destination.join("bad.bin").exists());
    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn trust_is_reused_on_second_transfer_without_code() {
    let root = tempfile::tempdir().unwrap();
    let (mut receiver, receiver_peer, _destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");
    let source = sender.dir.path().join("first.txt");
    std::fs::write(&source, b"one").unwrap();
    let sources = collect_sources(vec![source.clone()]).await.unwrap();

    let code = receiver.handle.pairing_code();
    let mut config = make_sender(
        &sender,
        receiver.handle.local_addr(),
        &receiver_peer,
        Some(code),
    );
    let (tx, _rx) = unbounded_channel();
    send_sources(&config, None, sources, tx, CancellationToken::new())
        .await
        .unwrap();
    assert!(sender
        .trust
        .is_trusted(&receiver_peer.identity.fingerprint()));

    std::fs::write(&source, b"two").unwrap();
    let sources = collect_sources(vec![source]).await.unwrap();
    config.pairing_code = None;
    let (tx, _rx) = unbounded_channel();
    let summary = send_sources(&config, None, sources, tx, CancellationToken::new())
        .await
        .expect("trusted session should not require a code");
    assert_eq!(summary.files.len(), 1);
    let _ = next_event(&mut receiver.events, Duration::from_secs(5)).await;
    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn source_changes_are_detected_before_sending() {
    let root = tempfile::tempdir().unwrap();
    let (receiver, receiver_peer, _destination) =
        start_receiver(root.path(), "Receiver", true).await;
    let sender = Peer::new(root.path(), "sender");
    let source = sender.dir.path().join("mutable.txt");
    std::fs::write(&source, b"version one").unwrap();
    let sources = collect_sources(vec![source.clone()]).await.unwrap();

    std::thread::sleep(Duration::from_millis(10));
    std::fs::write(&source, b"version two!").unwrap();
    std::fs::metadata(&source).unwrap();

    let code = receiver.handle.pairing_code();
    let config = make_sender(
        &sender,
        receiver.handle.local_addr(),
        &receiver_peer,
        Some(code),
    );
    let (tx, _rx) = unbounded_channel();
    let error = send_sources(&config, None, sources, tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransferError::SourceChanged(_)),
        "unexpected error: {error}"
    );
    receiver.handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn receiver_declining_transfer_stops_it() {
    let root = tempfile::tempdir().unwrap();
    let (receiver, receiver_peer, _destination) =
        start_receiver(root.path(), "Receiver", false).await;
    let sender = Peer::new(root.path(), "sender");
    let source = sender.dir.path().join("declined.txt");
    std::fs::write(&source, b"nope").unwrap();
    let sources = collect_sources(vec![source]).await.unwrap();

    let code = receiver.handle.pairing_code();
    let config = make_sender(
        &sender,
        receiver.handle.local_addr(),
        &receiver_peer,
        Some(code),
    );
    let (tx, _rx) = unbounded_channel();
    let error = send_sources(&config, None, sources, tx, CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            TransferError::PairingRejected(_) | TransferError::TransferRejected(_)
        ),
        "unexpected error: {error}"
    );
    assert!(!receiver.auto_approve.load(Ordering::SeqCst));
    receiver.handle.shutdown().await;
}
