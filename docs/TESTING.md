# Testing

## Automated tests

Command:

```
cargo test --workspace
```

Result of the recorded run: **48 tests, 0 failures** (plus two empty test harnesses for binaries).

| Suite | Count | Covers |
|---|---|---|
| `shadow-protocol` | 16 | frame roundtrip, oversized frames rejected before allocation, bad magic/flags/kind, empty control frames, exact control envelope, every variant has a payload, unknown fields, valid/invalid HELLO, path traversal, duplicate paths, total size mismatch, bad chunk length/hash, protocol downgrade, absurd file counts |
| `shadow-crypto` | 11 | identity persistence and uniqueness, empty identity rejection, code alphabet/normalization, proof binds both fingerprints and both nonces, wrong code rejection, session rotation, trust store roundtrip/revoke/corruption/invalid fingerprint, TLS config construction |
| `shadow-discovery` | 2 | strict beacon validation/unknown fields; real loopback advertise + discover |
| `shadow-transfer` (unit) | 8 | path acceptance/rejection, collision-safe reservation, selection-root preservation, nested/Unicode collection, symlink rejection, duplicate name rejection, byte formatting |
| `tests/e2e` | 10 | see below |

## End-to-end suite (real TCP, TLS, filesystem)

1. **Paired transfer delivers verified files** — pairing, 5 files + nested folder + Unicode name + empty file, receiver trust and sender trust updated, every delivered file byte-compared, receiver reports the correct file count/bytes, progress reaches `Completed`.
2. **Wrong pairing code** — rejected, no trust entries created, then the correct code succeeds (rate limiter did not lock a legitimate user out after one failure).
3. **Untrusted sender without code** — fails with `PairingRequired`, nothing written.
4. **Cancelled transfer resumes** — 64 MiB file, cancel after >1 MiB, receiver reports interruption, resend with the same transfer id resumes from a verified prefix, reused bytes match `resumed_from`, final bytes equal the source.
5. **Path traversal from a raw TLS client** — a trusted raw client sends `../evil.txt`; receiver returns code 400 and nothing is written outside the destination.
6. **Corrupt chunk hash** — trusted raw client sends a data frame whose hash does not match; receiver rejects it, no final file exists.
7. **Trust is reused** — second send works without a pairing code.
8. **Source changes** — file modified after collection is detected before sending; transfer fails with `SourceChanged`.
9. **Declined receiver** — the sender receives a pairing/transfer rejection and the transfer stops.
10. **Standalone pairing** — `pair_with` establishes trust and closes cleanly; a following send with no pairing code succeeds and delivers the file.

Security-negative tests (traversal, corrupt chunks, untrusted clients, bad frames) are part of this suite, not a separate mock harness.

## Manual UI verification performed

- **Windows receive:** a real 12 MiB file named `GUI-Test-文件.bin` was sent from the headless `send_demo` engine to the running Windows GUI. The UI displayed `Verified`, `12.0 MiB / 12.0 MiB`, speed and `BLAKE3 verified · 0.2s`. The file appeared in `C:\Users\<user>\Downloads\ShadowTransfer` with an identical SHA-256. Pairing and transfer approval dialogs were visually verified (name + fingerprint). The pairing code rotated after pairing.
- **Windows send:** the GUI (debug autosend hook, same code path as the Send button) sent a 12 MiB file to a headless auto-approving receiver. UI showed `Verified`, average 66.4 MB/s, `BLAKE3 verified on receiver`. Received SHA-256 matched exactly.
- **M2 premium UI:** Windows Home/Send/Receive/Activity/Trusted/Settings/Advanced screens compiled and were visually inspected at 1160×800; Android debug APK builds with bottom navigation, Home actions, Activity and Settings. The retheming has not been exercised on the physical tablet.
- **Receive mode crash fix discovered by UI testing:** `tokio::spawn` was called from the egui thread; this crashed receive mode. Fixed to use the application runtime and re-verified with a real receive.
- **Discovery/UX regression found with the real tablet on the LAN:** UDP beacons from the SM-T505 were confirmed arriving on Windows, but the code input was hidden inside a later modal and the port field accepted arbitrary text; the user pasted the code there. Fixed with a standalone Pair action, separate Auto Discovery / Manual Connection cards each with its own code field and button, numeric-only port, explicit "code needed" badge and "tap Approve on the tablet" guidance. The fixed build auto-discovered SM-T505 in the launch scan.

## Not tested (and why)

- **Physical Samsung Galaxy Tab A7:** the app was installed and used on a real SM-T505. Windows paired with it and transferred a 449 MB file and a 1.845 GB APK, both acknowledged as verified. Observed Wi-Fi throughput was ~15 MB/s on a high-latency 5 GHz link. Automated device tests, battery profiling, long background/doze runs and the latest speed-tuning build remain unverified on hardware.
- Wi-Fi throughput/pairing latency on real hardware.
- The installer was compiled but not executed on a clean machine.
- The redesigned standalone-pairing Windows flow was compiled and the pairing API was E2E-tested; the final button-driven approval path against the physical tablet still requires the tablet-side Approve tap.
- Multiple simultaneous receiver connections (limit is 4; tested sequentially).
- Huge folders near the 100,000-file cap.
- IPv6 (not implemented).
- Android 17/API 37 local-network runtime permission (only declared).
- Key-storage hardening (Keystore/DPAPI not wired).

## UI smoke-test hooks

Documented in WINDOWS.md. Debug-only hooks are compiled out of release behavior: `SHADOW_UI_AUTO_APPROVE`, `SHADOW_UI_DEMO_FILES`, `SHADOW_UI_AUTOSEND_*`. `SHADOW_UI_AUTOSTART_RECEIVE` and `SHADOW_UI_TOPMOST` exist in all builds for reproducible screenshots; they do not change the protocol or security logic.

## Manual tablet test procedure (next hardware step)

1. `adb install -r apps/android/app/build/outputs/apk/release/app-release.apk`
2. On the tablet: grant notifications, choose a Downloads subfolder with the SAF picker, enable Receive, note the code.
3. On Windows: enable Receive or use the sender; scan or enter the tablet IP:47831; enter the code; approve on the tablet.
4. Send a small file, then a multi-folder selection with Unicode names, then a 5 GB file; verify:
   - the tablet UI shows `BLAKE3 verified` only at completion;
   - file contents match (`adb shell sha256sum` on a folder that adb can read, or compare on-device);
   - no `.part` remnants after success; resume after disconnecting Wi-Fi mid-file;
   - battery/CPU via `adb shell dumpsys batterystats` and `top`;
   - rotate the tablet, background the app, kill the screen, confirm the foreground service keeps the receive alive as intended.
5. Record Wi-Fi vs raw `iperf3` throughput on the same LAN for a fair comparison.

## Reproducing local tests

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p shadow-e2e -- --test-threads=1 --nocapture
```

The E2E test that checks resume uses a 64 MiB file and is timing-sensitive on very fast loopback; if it ever finishes before cancellation, increase the file size in `tests/e2e/tests/transfer.rs`.
