//! JNI surface for the Android application. All transfer, TLS, pairing and
//! hashing work happens in Rust; Kotlin owns UI, SAF storage and lifecycle.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jstring};
use jni::JNIEnv;
use serde_json::{json, Value};
use shadow_discovery::AdvertiseConfig;
use shadow_transfer::{
    ApprovalKind, Direction, ProgressEvent, ReceiverConfig, ReceiverEvent, ReceiverHandle,
    SenderConfig, TransferPhase,
};
use tokio_util::sync::CancellationToken;

const RECEIVE_PORT: u16 = 47831;

struct Engine {
    runtime: tokio::runtime::Runtime,
    identity: Arc<shadow_crypto::Identity>,
    trust: Arc<shadow_crypto::TrustStore>,
    events: Mutex<VecDeque<Value>>,
    device_name: Mutex<String>,
    receiver: Mutex<Option<Arc<ReceiverHandle>>>,
    advertiser: Mutex<Option<CancellationToken>>,
    transfers: Mutex<HashMap<String, CancellationToken>>,
    scan: Mutex<Option<CancellationToken>>,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

fn engine() -> Option<Arc<Engine>> {
    ENGINE.get().cloned()
}

impl Engine {
    fn push(&self, event: Value) {
        let mut queue = self.events.lock().unwrap_or_else(|p| p.into_inner());
        if queue.len() >= 512 {
            queue.pop_front();
        }
        queue.push_back(event);
    }

    fn name(&self) -> String {
        self.device_name
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

fn init_engine(dir: PathBuf, device_name: String) -> Result<Arc<Engine>, String> {
    if let Some(existing) = engine() {
        return Ok(existing);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create data dir: {e}"))?;
    let identity = Arc::new(
        shadow_crypto::Identity::load_or_create(&dir).map_err(|e| format!("identity: {e}"))?,
    );
    let trust = Arc::new(
        shadow_crypto::TrustStore::load(dir.join("trusted.json"))
            .map_err(|e| format!("trust store: {e}"))?,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let engine = Arc::new(Engine {
        runtime,
        identity,
        trust,
        events: Mutex::new(VecDeque::new()),
        device_name: Mutex::new(device_name),
        receiver: Mutex::new(None),
        advertiser: Mutex::new(None),
        transfers: Mutex::new(HashMap::new()),
        scan: Mutex::new(None),
    });
    ENGINE
        .set(Arc::clone(&engine))
        .map_err(|_| "engine already initialized".to_string())?;
    Ok(engine)
}

fn throw(env: &mut JNIEnv, message: &str) {
    let _ = env.throw_new("java/lang/RuntimeException", message);
}

fn string_result(env: &mut JNIEnv, result: Result<String, String>) -> jstring {
    match result {
        Ok(value) => env
            .new_string(value)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        Err(err) => {
            throw(env, &err);
            std::ptr::null_mut()
        }
    }
}

fn guarded_string<F>(env: &mut JNIEnv, action: F) -> jstring
where
    F: FnOnce(&mut JNIEnv) -> Result<String, String> + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(env))) {
        Ok(result) => string_result(env, result),
        Err(_) => {
            throw(env, "native panic");
            std::ptr::null_mut()
        }
    }
}

fn guarded_bool<F>(env: &mut JNIEnv, action: F) -> jboolean
where
    F: FnOnce(&mut JNIEnv) -> Result<bool, String> + std::panic::UnwindSafe,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(env))) {
        Ok(Ok(value)) => jboolean::from(value),
        Ok(Err(err)) => {
            throw(env, &err);
            jboolean::from(false)
        }
        Err(_) => {
            throw(env, "native panic");
            jboolean::from(false)
        }
    }
}

fn read_string(env: &mut JNIEnv, value: &JString) -> Result<String, String> {
    env.get_string(value)
        .map(|s| s.into())
        .map_err(|e| format!("invalid string: {e}"))
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeInit(
    mut env: JNIEnv,
    _class: JClass,
    files_dir: JString,
    device_name: JString,
) -> jstring {
    guarded_string(&mut env, |env| {
        let dir = read_string(env, &files_dir)?;
        let name = read_string(env, &device_name)?;
        let engine = init_engine(PathBuf::from(dir), name)?;
        Ok(json!({
            "fingerprint": engine.identity.fingerprint_hex(),
            "deviceName": engine.name(),
        })
        .to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeFingerprint(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded_string(&mut env, |_env| {
        engine()
            .map(|e| e.identity.fingerprint_hex())
            .ok_or_else(|| "engine not initialized".to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeSetDeviceName(
    mut env: JNIEnv,
    _class: JClass,
    name: JString,
) -> jboolean {
    guarded_bool(&mut env, |env| {
        let name = read_string(env, &name)?;
        if name.trim().is_empty() || name.chars().count() > 64 {
            return Err("device name must be 1-64 characters".into());
        }
        let engine = engine().ok_or("engine not initialized")?;
        *engine.device_name.lock().unwrap_or_else(|p| p.into_inner()) = name;
        Ok(true)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeStartReceiver(
    mut env: JNIEnv,
    _class: JClass,
    destination: JString,
    device_name: JString,
) -> jboolean {
    guarded_bool(&mut env, |env| {
        let destination = read_string(env, &destination)?;
        let name = read_string(env, &device_name)?;
        let engine = engine().ok_or("engine not initialized")?;
        if engine
            .receiver
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            return Ok(true);
        }
        *engine.device_name.lock().unwrap_or_else(|p| p.into_inner()) = name.clone();
        let config = ReceiverConfig::new(
            std::net::SocketAddr::from(([0, 0, 0, 0], RECEIVE_PORT)),
            PathBuf::from(destination),
            name.clone(),
        );
        let identity = Arc::clone(&engine.identity);
        let trust = Arc::clone(&engine.trust);
        let task_engine = Arc::clone(&engine);
        engine.runtime.spawn(async move {
            match shadow_transfer::start(config, identity, trust).await {
                Ok((handle, mut events)) => {
                    let token = CancellationToken::new();
                    *task_engine
                        .advertiser
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = Some(token.clone());
                    let advertise = AdvertiseConfig::new(
                        handle.device_name().to_string(),
                        handle.local_addr().port(),
                        task_engine.identity.fingerprint_hex(),
                    );
                    tokio::spawn(async move {
                        let _ = shadow_discovery::advertise(advertise, token).await;
                    });
                    *task_engine
                        .receiver
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(handle));
                    task_engine.push(json!({
                        "type": "receiver_started",
                        "port": RECEIVE_PORT,
                    }));
                    while let Some(event) = events.recv().await {
                        task_engine.push(receiver_event_json(&event));
                    }
                }
                Err(err) => task_engine.push(json!({
                    "type": "receiver_failed",
                    "error": err.to_string(),
                })),
            }
        });
        Ok(true)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeStopReceiver(
    mut env: JNIEnv,
    _class: JClass,
) -> jboolean {
    guarded_bool(&mut env, |_env| {
        let engine = engine().ok_or("engine not initialized")?;
        if let Some(token) = engine
            .advertiser
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            token.cancel();
        }
        let handle = engine
            .receiver
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(handle) = handle {
            engine.runtime.spawn(async move {
                handle.shutdown().await;
            });
        }
        engine.push(json!({"type": "stopped"}));
        Ok(true)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativePairingCode(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded_string(&mut env, |_env| {
        let engine = engine().ok_or("engine not initialized")?;
        let handle = engine
            .receiver
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or("receiver is not running")?;
        Ok(handle.pairing_code())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeNewPairingCode(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded_string(&mut env, |_env| {
        let engine = engine().ok_or("engine not initialized")?;
        let handle = engine
            .receiver
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or("receiver is not running")?;
        Ok(handle.regenerate_pairing_code())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeRespond(
    mut env: JNIEnv,
    _class: JClass,
    approval_id: JString,
    accept: jboolean,
) -> jboolean {
    guarded_bool(&mut env, |env| {
        let id = read_string(env, &approval_id)?;
        let engine = engine().ok_or("engine not initialized")?;
        let handle = engine
            .receiver
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or("receiver is not running")?;
        Ok(handle.respond(&id, accept != jboolean::from(false)))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeListTrusted(
    mut env: JNIEnv,
    _class: JClass,
) -> jstring {
    guarded_string(&mut env, |_env| {
        let engine = engine().ok_or("engine not initialized")?;
        let devices = engine.trust.list().map_err(|e| e.to_string())?;
        let array: Vec<Value> = devices
            .into_iter()
            .map(|device| {
                json!({
                    "name": device.name,
                    "fingerprint": device.fingerprint,
                    "addedAt": device.added_at,
                    "lastSeen": device.last_seen,
                })
            })
            .collect();
        Ok(json!(array).to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeRevokeTrusted(
    mut env: JNIEnv,
    _class: JClass,
    fingerprint: JString,
) -> jboolean {
    guarded_bool(&mut env, |env| {
        let value = read_string(env, &fingerprint)?;
        let bytes = hex::decode(value).map_err(|_| "invalid fingerprint encoding".to_string())?;
        let fingerprint: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "fingerprint must be 32 bytes".to_string())?;
        let engine = engine().ok_or("engine not initialized")?;
        engine.trust.revoke(&fingerprint).map_err(|e| e.to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeCancelTransfer(
    mut env: JNIEnv,
    _class: JClass,
    transfer_id: JString,
) -> jboolean {
    guarded_bool(&mut env, |env| {
        let id = read_string(env, &transfer_id)?;
        let engine = engine().ok_or("engine not initialized")?;
        if let Some(token) = engine
            .transfers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
        {
            token.cancel();
            return Ok(true);
        }
        if let Some(handle) = engine
            .receiver
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            return Ok(handle.cancel_transfer(&id));
        }
        Ok(false)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeSendFiles(
    mut env: JNIEnv,
    _class: JClass,
    host: JString,
    port: jint,
    paths_json: JString,
    pairing_code: JString,
) -> jstring {
    guarded_string(&mut env, |env| {
        let host = read_string(env, &host)?;
        let paths_json = read_string(env, &paths_json)?;
        let code = read_string(env, &pairing_code)?;
        let paths: Vec<String> =
            serde_json::from_str(&paths_json).map_err(|e| format!("invalid paths: {e}"))?;
        if paths.is_empty() {
            return Err("no files selected".into());
        }
        if !(1..=65535).contains(&port) {
            return Err("invalid port".into());
        }
        let address = format!("{host}:{port}")
            .parse()
            .map_err(|_| "invalid address".to_string())?;
        let engine = engine().ok_or("engine not initialized")?;
        let transfer_id = shadow_transfer::generate_transfer_id().map_err(|e| e.to_string())?;
        let cancel = CancellationToken::new();
        engine
            .transfers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(transfer_id.clone(), cancel.clone());
        engine.push(json!({
            "type": "send_started",
            "transfer_id": transfer_id,
            "peer": host,
        }));
        let task_engine = Arc::clone(&engine);
        let task_id = transfer_id.clone();
        let device_name = engine.name();
        let identity = Arc::clone(&engine.identity);
        let trust = Arc::clone(&engine.trust);
        engine.runtime.spawn(async move {
            let mut config = SenderConfig::new(address, identity, trust, device_name);
            config.pairing_code = if code.trim().is_empty() {
                None
            } else {
                Some(code)
            };
            config.approval_timeout = Duration::from_secs(180);
            let sources = match shadow_transfer::collect_sources(
                paths.into_iter().map(PathBuf::from).collect(),
            )
            .await
            {
                Ok(sources) => sources,
                Err(err) => {
                    task_engine.push(json!({
                        "type": "send_failed",
                        "transfer_id": task_id,
                        "error": err.to_string(),
                    }));
                    task_engine
                        .transfers
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&task_id);
                    return;
                }
            };
            let (progress_tx, mut progress_rx) =
                tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
            let forward_engine = Arc::clone(&task_engine);
            let forwarder = tokio::spawn(async move {
                while let Some(event) = progress_rx.recv().await {
                    forward_engine.push(progress_json(&event));
                }
            });
            let result = shadow_transfer::send_sources(
                &config,
                Some(task_id.clone()),
                sources,
                progress_tx,
                cancel,
            )
            .await;
            let _ = forwarder.await;
            match result {
                Ok(summary) => task_engine.push(json!({
                    "type": "send_finished",
                    "transfer_id": task_id,
                    "peer": summary.peer_name,
                    "files": summary.files.len(),
                    "bytes": summary.bytes_sent + summary.bytes_reused,
                    "reused": summary.bytes_reused,
                })),
                Err(err) => task_engine.push(json!({
                    "type": "send_failed",
                    "transfer_id": task_id,
                    "error": err.to_string(),
                })),
            }
            task_engine
                .transfers
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&task_id);
        });
        Ok(transfer_id)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativeDiscover(
    mut env: JNIEnv,
    _class: JClass,
) -> jboolean {
    guarded_bool(&mut env, |_env| {
        let engine = engine().ok_or("engine not initialized")?;
        if engine
            .scan
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            return Ok(true);
        }
        let cancel = CancellationToken::new();
        *engine.scan.lock().unwrap_or_else(|p| p.into_inner()) = Some(cancel.clone());
        let task_engine = Arc::clone(&engine);
        engine.runtime.spawn(async move {
            task_engine.push(json!({"type": "scanning"}));
            match shadow_discovery::discover_for(Duration::from_secs(3), cancel).await {
                Ok(devices) => {
                    let list: Vec<Value> = devices
                        .iter()
                        .map(|device| {
                            json!({
                                "name": device.name,
                                "addr": device.addr.to_string(),
                                "fingerprint": device.fingerprint,
                            })
                        })
                        .collect();
                    task_engine.push(json!({"type": "devices", "devices": list}));
                }
                Err(err) => task_engine.push(json!({
                    "type": "scan_failed",
                    "error": err.to_string(),
                })),
            }
            *task_engine.scan.lock().unwrap_or_else(|p| p.into_inner()) = None;
        });
        Ok(true)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_shadowtransfer_app_ShadowEngine_nativePollEvent(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let event = engine().and_then(|engine| {
        engine
            .events
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front()
    });
    match event {
        Some(value) => env
            .new_string(value.to_string())
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}

fn progress_json(event: &ProgressEvent) -> Value {
    json!({
        "type": "progress",
        "transfer_id": event.transfer_id,
        "direction": match event.direction {
            Direction::Send => "send",
            Direction::Receive => "receive",
        },
        "phase": phase_name(event.phase),
        "file_name": event.file_name,
        "file_index": event.file_index,
        "file_count": event.file_count,
        "file_bytes": event.file_bytes,
        "file_size": event.file_size,
        "total_done": event.total_bytes_done,
        "total_bytes": event.total_bytes,
        "speed_bps": event.speed_bps,
        "avg_bps": event.avg_bps,
        "eta_secs": event.eta_secs,
    })
}

fn phase_name(phase: TransferPhase) -> &'static str {
    match phase {
        TransferPhase::Connecting => "connecting",
        TransferPhase::Pairing => "pairing",
        TransferPhase::Transferring => "transferring",
        TransferPhase::Verifying => "verifying",
        TransferPhase::Completed => "completed",
        TransferPhase::Failed => "failed",
        TransferPhase::Cancelled => "cancelled",
    }
}

fn receiver_event_json(event: &ReceiverEvent) -> Value {
    match event {
        ReceiverEvent::Listening { addr } => json!({
            "type": "listening",
            "port": addr.port(),
        }),
        ReceiverEvent::PairingCode {
            code,
            expires_in_secs,
        } => json!({
            "type": "pairing_code",
            "code": code,
            "expires": expires_in_secs,
        }),
        ReceiverEvent::ApprovalNeeded(approval) => match &approval.kind {
            ApprovalKind::Pairing {
                device_name,
                fingerprint,
            } => json!({
                "type": "approval",
                "id": approval.id,
                "kind": "pairing",
                "device": device_name,
                "fingerprint": fingerprint,
            }),
            ApprovalKind::Transfer {
                sender_name,
                transfer_id,
                file_count,
                total_bytes,
                preview,
            } => json!({
                "type": "approval",
                "id": approval.id,
                "kind": "transfer",
                "device": sender_name,
                "transfer_id": transfer_id,
                "file_count": file_count,
                "total_bytes": total_bytes,
                "preview": preview,
            }),
        },
        ReceiverEvent::Progress(event) => progress_json(event),
        ReceiverEvent::FileReceived {
            transfer_id,
            file_name,
            destination,
            bytes,
        } => json!({
            "type": "file_received",
            "transfer_id": transfer_id,
            "file_name": file_name,
            "path": destination.to_string_lossy(),
            "bytes": bytes,
        }),
        ReceiverEvent::TransferFinished {
            transfer_id,
            files,
            bytes,
            duration_secs,
        } => json!({
            "type": "finished",
            "transfer_id": transfer_id,
            "files": files,
            "bytes": bytes,
            "duration": duration_secs,
        }),
        ReceiverEvent::TransferFailed { transfer_id, error } => json!({
            "type": "failed",
            "transfer_id": transfer_id,
            "error": error,
        }),
        ReceiverEvent::Stopped => json!({"type": "stopped"}),
    }
}
