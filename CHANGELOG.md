# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Windows foreground auto-discovery with online/offline/last-seen state and trusted-device auto-selection.
- Android foreground auto-discovery and a trusted-device list with revoke.
- Regression test for the device presence window.

## [0.1.0] - 2026-11-07

### Added

- Shared Rust transfer engine used by Windows and Android.
- TLS 1.3-only transport with mutual certificates, ALPN, and SHA-256
  certificate-fingerprint pinning.
- One-time 8-character pairing codes (about 40 bits) with BLAKE3 keyed proofs
  bound to both device fingerprints and both nonces, explicit receiver
  approval, single-use codes, 5-minute expiry, and pairing rate limiting.
- Persistent trusted-device store with revocation in the Windows settings.
- Streaming transfers with bounded memory, adaptive 256 KiB–4 MiB chunks,
  per-chunk BLAKE3, full-file BLAKE3 verification, and collision-safe
  finalization that never overwrites existing files.
- Resume from a verified prefix after cancellation or connection loss,
  including revalidation of files completed in an earlier session.
- Cancellation from either side, automatic retry on transient send failures,
  and manual retry with the same transfer ID.
- UDP broadcast discovery while receiving, manual IP fallback, and a separate
  auto-discovery/manual-connection UI.
- Windows egui application with drag-and-drop, queue, live speed/ETA, history,
  settings, receive mode, approval dialogs, and portable/installer packaging.
- Android Compose application with SAF storage staging/import, receive mode,
  foreground data-sync service, pairing-code countdown, transfer approvals,
  discovery/manual sending, and a Rust JNI engine.
- Release APK for `arm64-v8a`, `armeabi-v7a`, and `x86_64`.
- Documentation set: architecture, protocol, security, networking, Windows,
  Android, build, testing, performance, troubleshooting.
- Automated tests: 47 total, including real TLS/socket/filesystem E2E tests.

### Security

- No plaintext fallback and no silent reduction of protection.
- Strict bounded framing, message validation, path validation and file limits.
- Received files are never executed automatically.
- Pairing approvals and transfer approvals are explicit on the receiver.

### Performance

- 8 MiB best-effort TCP buffers on both peers.
- Fresh transfers are verified from the receiver's incremental BLAKE3 state,
  avoiding a second full-file read; resumed files are re-read and re-hashed.
- Android holds a high-performance Wi-Fi lock only while a transfer is active.
- Loopback benchmark after tuning: 5 GiB at 324.9 MiB/s; 100 MiB up to
  490 MiB/s. Real Wi-Fi throughput is link-bound.

### Known limitations

- Android identity keys are stored in app-private storage, not wrapped by
  Android Keystore; Windows keys are user-profile protected, not DPAPI-wrapped.
- Windows binaries and the installer are unsigned.
- The release APK is signed with a locally generated keystore, not a Play Store
  signing setup.
- IPv4 LAN only; no QR pairing, compression, QUIC or parallel streams.
- Android battery/background behavior and the latest speed-tuning APK have not
  been re-measured on the physical tablet.
- No automated Android device tests.

[0.1.0]: https://github.com/b7662777-hash/shadow-transfer/releases/tag/v0.1.0
