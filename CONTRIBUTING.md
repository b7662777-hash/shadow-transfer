# Contributing to Shadow Transfer

Thanks for taking the time to improve Shadow Transfer. This project takes security, data integrity and cross-platform correctness seriously; small, well-tested changes are preferred over large rewrites.

## Before you start

- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), [docs/PROTOCOL.md](docs/PROTOCOL.md) and [docs/SECURITY.md](docs/SECURITY.md).
- For security vulnerabilities, do **not** open a public issue. Follow [SECURITY.md](SECURITY.md).
- For large features or protocol changes, open an issue first so the design can be discussed.

## Development setup

Prerequisites:

- Rust stable (see `rust-toolchain.toml`), MSVC build tools on Windows.
- JDK 17 and Android SDK 34 + NDK 26.1.10909125 for the Android app.
- Gradle wrapper is included; `cargo-ndk` is only needed to rebuild the JNI library.

Quality gates (must pass before a PR):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Windows:

```bash
cargo build --release -p shadow-windows
```

Android:

```bash
cd apps/android/rust
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
  -o ../app/src/main/jniLibs --platform 26 build --release -p shadow-android
cd ..
./gradlew :app:assembleDebug
```

Full instructions are in [docs/BUILD.md](docs/BUILD.md).

## Code standards

- Rust: `rustfmt` + `clippy -D warnings`; no unexplained `unsafe`; errors are typed, no panics for recoverable failures.
- Kotlin: Kotlin style, Compose-first, no blocking work on the main thread.
- No new dependencies without a concrete need; document why a crate or library is used.
- No secrets, keystores, signing passwords or personal network data in the repository or logs.
- All file paths and network input must remain validated and bounded.
- Do not weaken TLS, pinning, pairing, hashing or finalization behavior for speed.

## Protocol changes

The protocol is versioned and both clients must implement exactly the same wire format. Any change requires:

1. A version bump or a backwards-compatible negotiation strategy.
2. Updates to `docs/PROTOCOL.md`.
3. Validation tests for malformed/oversized input.
4. An end-to-end test covering both directions.
5. A security review of replay, downgrade and trust implications.

## Tests

New behavior needs tests. Prefer targeted unit tests plus a real-socket E2E test for anything touching the transfer/session layer. See [docs/TESTING.md](docs/TESTING.md).

## Commits and pull requests

- Keep commits focused; imperative subjects such as `fix: resend chunk after resume reset`.
- Describe the problem, the chosen approach, and the tests run.
- Fill in the PR checklist.
- Keep generated build output, APKs, EXEs and local keys out of commits.
- The CI must be green.

## Licensing

By contributing, you agree that your contribution is licensed under the MIT License, as described in [LICENSE](LICENSE).
