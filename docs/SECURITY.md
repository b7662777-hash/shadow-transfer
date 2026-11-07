# Security

## Scope and assets

Shadow Transfer protects files in transit between two devices the user controls on the same LAN. Assets:

- file contents and names;
- device identities and the trusted-device list;
- pairing codes;
- the guarantee that a file shown as complete really is the file the sender sent.

## Threat model

Considered:

- A passive attacker on the LAN reading traffic.
- An active attacker able to intercept, modify, replay or inject TCP/UDP traffic.
- An unauthorized device attempting to connect, pair or brute-force a code.
- A malicious sender trying path traversal, oversized messages, malformed metadata, fake lengths, resource exhaustion or a path outside the destination.
- Replayed pairing proofs and replayed sessions.
- Man-in-the-middle attackers, including those relaying discovery beacons.

Not considered:

- A compromised operating system or a user with filesystem/debug access to the device profile.
- Denial of service by flooding a LAN or exhausting the TCP listener; connection limits and timeouts reduce but do not eliminate this.
- Metadata privacy: device names and fingerprints are broadcast unauthenticated while receive mode is enabled.
- Traffic analysis (sizes and timing are visible to a network observer, as with any TLS connection).

## Identity

Each device generates an ECDSA P-256 key pair and a self-signed certificate at first launch. The device identity is the SHA-256 fingerprint of the DER certificate. The private key and certificate are persisted in the app's private directory:

- Windows: `%LOCALAPPDATA%\ShadowTransfer\identity.key.der` / `identity.cert.der`.
- Android: app-private `filesDir` (Rust-managed).

No certificate authority is involved. There are no hardcoded secrets, passwords or shared keys.

**Known limitation:** the Android key is not wrapped by Android Keystore and the Windows key is not DPAPI-protected; it relies on OS profile/app-sandbox permissions. This is a deliberate scope decision for this release, not a claim of hardware-backed protection.

## TLS 1.3 and pinning

- rustls with the `ring` provider; TLS 1.3 only; client certificates are mandatory; TLS resumption tickets disabled on the server.
- ALPN is enforced on both sides.
- Both peers use a custom certificate verifier because this is a private peer-to-peer protocol with self-signed certificates:
  - the TLS handshake signature is still fully verified by rustls;
  - the certificate chain is intentionally not WebPKI-validated;
  - the session layer captures the peer certificate fingerprint after the handshake and **must** pin it before reading any application message.
- The receiver checks the client fingerprint against its trust store before sending `HELLO_ACK`.
- The sender checks the fingerprint declared in `HELLO_ACK` against the certificate captured from TLS and against its own trust store.

Refusal rules:

- Unknown client on a receiver → pairing path only.
- Unknown server on a sender → never proceed, even if the server claims to trust the client. This prevents a reinstall from silently trusting an attacker.
- Pairing unavailable or code missing → connection refused; no plaintext fallback ever occurs.
- Any verification failure stops the session; protection is never silently reduced.

This design is only secure because pinning is mandatory. The `AcceptAnyCertificate` verifier is private to the crypto crate and is wired only into configs used by the session layer. Code review of that invariant matters.

## Pairing

Code properties:

- 8 characters from `ABCDEFGHJKMNPQRSTUVWXYZ23456789` (31 symbols) → 31^8 ≈ 8.5 × 10^11 combinations ≈ 39.6 bits.
- Generated with `getrandom` and rejection sampling (no modulo bias).
- 300-second lifetime, single use, rotated after successful pairing or on expiry.
- Shown as a large monospace string; the user may add dashes/spaces, which are normalized.

Proof:

```
key   = blake3::derive_key("shadow-transfer pairing v1", code)
input = "SHADOW-PAIR-v1" || client_cert_fp || server_cert_fp ||
        client_nonce(16) || server_nonce(16)
proof = blake3::keyed_hash(key, input)
```

Both fingerprints and both fresh nonces (server nonce generated per connection) are bound. An attacker who relays messages between two victims cannot make the fingerprints and nonces agree. A recorded proof is useless on a new connection because the nonces change. Comparison is constant-time.

Even a correct proof does not auto-trust the device: the receiver displays the device name and fingerprint and requires an explicit approval. This is the primary defense against a leaked code.

Brute-force resistance:

- one proof attempt per connection; rejected connections are not retried automatically;
- 750 ms delay before a rejection;
- more than five failures in 60 seconds disables pairing until the window expires;
- at most four concurrent connections;
- code entropy makes online guessing infeasible even without rate limits.

On success both sides pin the other's certificate fingerprint in their local trust store. A trust store is loaded strictly: corrupt JSON or an invalid fingerprint is an error, never a silent reset. Windows exposes revocation in Settings; revoking removes the pin.

## File safety

- Relative paths are validated on the wire and again at the receiver with one shared cross-platform policy. Traversal, absolute paths, backslashes, empty components, control chars, reserved DOS names (`CON`, `NUL`, `COM1`, …), Windows-illegal characters and trailing dots/spaces are rejected. A rejected transfer never writes outside the destination.
- Files are written as `<file_id>.part` under `.shadow-transfer-state/<transfer_id>/`, which is created inside the destination and keyed by a validated 16-byte hex transfer id.
- Each chunk is BLAKE3-verified before it is written.
- Every chunk is hashed as it arrives. For a file started at offset 0 the receiver finalizes that same incremental BLAKE3 state, compares it with the sender's full hash, flushes and `sync_data`s the file, and only then finalizes it. A resumed file (offset > 0) has no valid incremental state and is read back completely and re-hashed before finalization. The incremental path still computes the receiver's own hash over every received byte; it avoids a redundant disk read, not verification.
- Finalization reserves the destination name with `create_new`, so an existing user file is never overwritten; collisions get ` (1)`, ` (2)`, etc. The verified partial is renamed over our own reservation.
- Received files are never opened by the app, never executed, and never registered with the shell.
- On any failure the partial and manifest remain for resume; they are not presented as complete. Stale state is removed after seven days.

## Resource exhaustion controls

- Frame lengths are checked before allocation; control frames ≤256 KiB; data ≤8 MiB.
- Messages have field-specific limits; file counts, path lengths and total byte sums are checked.
- Data-frame length and content hash must match the preceding `FILE_CHUNK`.
- One sequential file stream per connection; 4-connection semaphore; bounded buffers.
- Approval timeouts (180 s) and I/O timeouts prevent idle sessions from lingering forever.
- Pairing attempts are rate-limited globally.

## Integrity guarantees

A file is reported `verified` only after the receiver independently re-reads the entire finalized partial and matches both size and full BLAKE3 to the sender's `FILE_COMPLETE`. The sender counts a skipped, previously completed file only after hashing its own source and matching the receiver's stored hash. Progress and UI status never claim success before these checks.

## Logging and secrets

- Pairing codes are never written to logs.
- Certificate fingerprints, device names, sizes and errors may be logged locally; no telemetry leaves the device.
- Errors are surfaced through UI events and structured logs; recoverable failures are not silently suppressed.

## Verification

Security-relevant automated tests currently cover:

- invalid/short identity files, identity persistence, unique identities;
- pairing proof binding to both fingerprints and nonces, wrong code, normalization, code rotation;
- strict trust-store loading and revocation;
- path traversal and platform-hazard rejection, duplicate names;
- malformed envelopes, unknown fields, oversized frames, bad magic/flags;
- path traversal attempt from a raw TLS client, and corrupt chunk hash rejection.

See [TESTING.md](TESTING.md) for exact tests and commands.

## Reporting

This is an offline open-source project. If you find a vulnerability, open a private report with the maintainers rather than publishing exploit details. Include the protocol version, platform, and a reproducible trace if possible.
