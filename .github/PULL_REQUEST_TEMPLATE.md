## What changed

<!-- Describe the problem and the approach. -->

## Tests

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Android build (if Kotlin/JNI/UI changed)
- [ ] Windows manual smoke test (if UI changed)

## Security / integrity

- [ ] No secrets, keystores, personal IPs or private keys added
- [ ] No reduction in TLS, pinning, pairing, hashing, path validation or
      finalization guarantees
- [ ] Protocol documentation updated if the wire format changed
- [ ] Resume/cancel behavior covered if the transfer engine changed

## Screenshots / notes

<!-- Optional. Redact pairing codes and IP addresses. -->
