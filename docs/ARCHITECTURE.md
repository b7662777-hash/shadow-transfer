# Architecture

## Design goal

One transfer engine, two thin platform shells. All protocol, cryptography, file streaming, integrity verification and resume logic live in Rust and are shared byte-for-byte by Windows and Android. Platform code owns only UI, lifecycle, storage permissions and notification concerns.

## Crates

| Crate | Responsibility |
|---|---|
| `shadow-protocol` | 12-byte frame header, bounded payloads, JSON control messages, validation of every field |
| `shadow-crypto` | ECDSA P-256 device identity, self-signed certificate, TLS 1.3 client/server configs, certificate pin helpers, pairing proofs, trust store |
| `shadow-transfer` | client handshake and approval flow, transfer sender, receiver service, path sanitization, resume manifest, progress accounting, per-chunk and whole-file BLAKE3 |
| `shadow-discovery` | UDP broadcast beacon sender and short-window listener |
| `apps/windows` | egui/eframe desktop UI; owns the Tokio runtime and adapts engine events to UI state |
| `apps/android/rust` | JNI surface, engine singleton, Kotlin event queue |
| `apps/android/app` | Kotlin/Compose UI, SAF staging/import, foreground service, permission handling |
| `tests/e2e` | real sockets, real TLS, real files; no mocks |

## Windows runtime

```
egui (main thread) ──spawn──► Tokio runtime ──► transfer engine
       ▲                          │
       └──── AppEvent channel ◄────┘  (progress, approvals, completion, errors)
```

- The egui main thread never blocks on network or disk. Every operation is spawned on a two-worker Tokio runtime held by the app.
- A single unbounded channel carries `AppEvent`s into the UI: receiver events, send progress, send results, discovery results and source-collection results.
- The UI drains the channel once per frame and requests a 100 ms repaint while work is active. When idle, no polling occurs.
- Incoming approvals are queued; the UI shows one dialog at a time and answers the receiver's `oneshot` via `ReceiverHandle::respond`.
- Cancellation is represented by `CancellationToken`s retained by the UI per transfer.

## Receiver service

`shadow_transfer::start` binds a TCP listener, builds the rustls server config once, and spawns an accept loop with a 4-connection semaphore. Each connection:

1. TLS 1.3 handshake. Client certificates are mandatory. ALPN must equal `shadow-transfer/1`.
2. Capture the client certificate fingerprint.
3. `HELLO` / `HELLO_ACK`; decide trusted vs pairing path.
4. Trusted path: verify pin, send `SESSION_READY`.
   Pairing path: verify time-limited code proof, ask the user, persist both fingerprints, rotate the code, send `PAIR_ACCEPT` + `SESSION_READY`.
5. Await `TRANSFER_REQUEST`, ask the user, open `ReceiveState`, send resume entries.
6. Stream chunks, verify each chunk, verify the whole file after flush, reserve a collision-free final name, rename, acknowledge.

Every session is bounded by timeouts; failures send an error frame when possible and preserve partial state for resume.

## Receive state and resume

```
<destination>/.shadow-transfer-state/<transfer_id>/manifest.json
<destination>/.shadow-transfer-state/<transfer_id>/<file_id>.part
```

- The manifest records file ids, relative paths, sizes, source mtimes, completion status, final names and full BLAKE3 hashes.
- A request that does not exactly match the manifest (ids, paths, sizes, mtimes) resets that transfer's partial data. This prevents splicing a different source into old partials.
- For an incomplete file the receiver offers `offset = partial length` plus BLAKE3 of that prefix. The sender recomputes that prefix; a mismatch restarts the file from zero.
- A partially received file whose length already equals the file size is **not** offered as complete. It restarts, so verification is never skipped.
- Fresh files (offset 0) maintain an incremental BLAKE3 state while chunks are written; finalization uses it instead of re-reading the file. Resumed files (offset > 0) are fully re-read and re-hashed.
- Accepted TCP sockets get best-effort 8 MiB send/receive buffers on both platforms to keep a single stream usable on high-latency Wi-Fi.
- A file completed in an earlier session is revalidated (exists, size and full hash) before the receiver offers `completed = true`. The sender independently hashes its source; only then is the file skipped.
- Finalization reserves the destination name with `create_new` (never overwrites), then renames the verified partial over the reservation. This closes the check-then-rename race on all supported filesystems.
- Stale state older than seven days is removed when a receiver starts.

## File selection

- Files and folders are walked without following symlinks. Direct symlink selections are rejected.
- Each selection keeps its root name: selecting `album/` sends `album/...`; selecting `a.txt` sends `a.txt`.
- Names are validated against one cross-platform rule set (no traversal, no Windows-reserved names, no `<>:"|?*`, no trailing dots/spaces, bounded component length). Both platforms therefore accept the same manifest.
- Duplicate destination names within one selection are rejected rather than silently renamed.

## Progress and speed

- Sender and receiver use the same `ProgressTracker`; bytes are counted only after they are actually read, sent, or written.
- Speed is computed from a 3-second rolling window; average speed from transfer start. ETA is `remaining / rolling speed` and is absent when speed is too low to estimate.
- Events are throttled to 100 ms and always emitted for phase changes.

## Android runtime

```
Compose UI ── StateFlow ── ShadowRepository ── JNI ── Rust engine (Tokio, 2 workers)
      ▲                          │
      └──── poll native events ──┘  every 120 ms (background IO coroutine)
```

- The Rust engine is initialized once in `ShadowApplication`; identity and trust store live in app-private storage.
- `nativePollEvent` returns one JSON event or null. The repository translates events into immutable UI state, persists a 50-entry history, triggers SAF imports, and manages the foreground service notification.
- SAF is used for both directions:
  - **Receiving:** Rust writes and verifies into app staging (`incoming/`), then Kotlin copies each verified file into the user-selected document tree, creating folders as needed and avoiding name collisions. On success the staging copy is deleted.
  - **Sending:** Kotlin copies selected documents/folders into cache staging (`outgoing/batch-*`), then Rust streams from those real files. This is required because SAF URIs are not filesystem paths.
- A `dataSync` foreground service keeps the process alive while receive mode is enabled. No wake locks or Wi-Fi locks are held; a `MulticastLock` is acquired only for the explicit three-second device scan.
- Device discovery uses the shared Rust discovery crate; the Kotlin layer only handles the Android multicast lock and permission prompts.

## Threading and memory

- One sequential chunk stream per transfer; no parallel connections per file. Concurrency is limited to four receiver connections and the UI's send tasks.
- File buffers are bounded: data frames are capped at 8 MiB; the sender allocates one reusable buffer up to 4 MiB; receiver hashing uses a 256 KiB buffer.
- Files are never loaded whole into memory.
- Hashing completed files runs in `spawn_blocking` so the async runtime remains responsive.

## Deliberate architectural decisions

1. **UDP broadcast instead of mDNS** — avoids multicast complexity and platform NSD differences for a two-device LAN tool; beacons exist only while receiving. Manual IP is always available.
2. **App-level certificate pinning instead of a private CA/WebPKI** — no certificates to install, no hostname dependency, exact device identity. Security depends on mandatory pinning (documented) and the pairing proof.
3. **Sequential streaming instead of parallel chunks** — sustained throughput on a weak tablet, predictable memory, simpler integrity and resume semantics.
4. **TLS 1.3 only** — removes legacy protocol surface; both peers are shipped together.
5. **SAF staging rather than unrestricted storage permissions** — the app never requests broad storage access; verified data is copied through the document provider.
6. **Rust owns identity and trust** — identical security behavior on both platforms. Hardware-backed key wrapping is listed as a known limitation.
