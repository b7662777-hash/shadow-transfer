# Shadow Transfer Protocol v1

Both clients implement exactly this protocol. The Rust implementation is the canonical one.

## Transport

- TCP + TLS 1.3 only. TLS 1.2 and plaintext are never offered.
- ALPN: `shadow-transfer/1`.
- Both peers present a self-signed ECDSA P-256 certificate and require the other's certificate (mutual authentication).
- The receiver listens on TCP **47831** by default. The Windows settings screen can change the port; the Android build uses 47831.

## Framing

Every frame starts with a fixed 12-byte header:

| Offset | Size | Field | Rule |
|---|---|---|---|
| 0 | 4 | magic | ASCII `STX1` |
| 4 | 1 | frame version | must be `1` |
| 5 | 1 | kind | `0` control, `1` data |
| 6 | 2 | flags | must be zero (little-endian) |
| 8 | 4 | payload length | little-endian, bounded by kind |

Limits, checked **before** allocation:

| Kind | Maximum payload | Notes |
|---|---|---|
| Control | 256 KiB | JSON, must be non-empty |
| Data | 8 MiB | raw file bytes |

Unknown kind, wrong magic/version, non-zero flags, oversized or empty control frames are protocol errors; the connection closes. A data frame is never read unless a preceding control message declared its exact length.

## Control envelope

Control payloads are UTF-8 JSON objects with exactly two keys:

```json
{"type":"HELLO","payload":{"protocol":1,"device_name":"Workstation","nonce":"00112233445566778899aabbccddeeff"}}
```

- The envelope may contain only `type` and `payload`.
- Every payload struct rejects unknown fields.
- Every message is validated again on receipt (field lengths, hex encoding, path safety, total byte accounting).
- Unknown message types are rejected. Unexpected message types terminate the session.

## Message catalog

Direction: C→S means sender/client to receiver/server.

| Type | Dir | Payload fields | Validation |
|---|---|---|---|
| `HELLO` | C→S | `protocol`, `device_name`, `nonce` | protocol must equal 1; name 1–64 printable chars; nonce 16 bytes hex |
| `HELLO_ACK` | S→C | `protocol`, `device_name`, `fingerprint`, `nonce`, `client_trusted`, `pairing_available`, `pairing_expires_secs` | fingerprint = SHA-256 of server certificate, 32 bytes hex; nonce 16 bytes hex |
| `PAIR_REQUEST` | C→S | `device_name`, `fingerprint`, `proof` | fingerprint must equal the client certificate fingerprint; proof 32 bytes hex |
| `PAIR_ACCEPT` | S→C | `fingerprint` | must equal the server certificate fingerprint |
| `PAIR_REJECT` | S→C | `reason` | ≤512 chars |
| `SESSION_READY` | S→C | `session_id` | 16 bytes hex |
| `TRANSFER_REQUEST` | C→S | `transfer_id`, `sender_name`, `files[]`, `total_bytes` | id 16 bytes hex; 1..100000 files; unique safe relative paths; sum of sizes equals `total_bytes`; each size ≤ 2^50 |
| `TRANSFER_ACCEPT` | S→C | `entries[]` | each entry references a requested file; offsets ≤ size; hash 32 bytes hex |
| `TRANSFER_REJECT` | S→C | `reason` | ≤512 chars |
| `FILE_START` | C→S | `file_id`, `offset` | offset ≤ file size |
| `FILE_CHUNK` | C→S | `file_id`, `offset`, `len`, `hash` | `len` 1..8 MiB; `hash` = BLAKE3 of the following data frame |
| data frame | C→S | raw bytes | length must equal the declared `len`; hash must match |
| `FILE_COMPLETE` | C→S | `file_id`, `size`, `hash` | full BLAKE3 of source file |
| `FILE_ACK` | S→C | `file_id`, `verified`, `reason` | sent only after receiver-side full re-hash and finalization |
| `TRANSFER_COMPLETE` | C→S | `transfer_id`, `files_verified` | |
| `TRANSFER_ACK` | S→C | `transfer_id` | sent only when every requested file is accounted for |
| `CANCEL` | both | `reason` | ≤512 chars |
| `ERROR` | both | `code`, `message` | message ≤2048 chars |
| `PING` / `PONG` | both | `nonce` | 16 bytes hex |

File metadata in `TRANSFER_REQUEST`:

```json
{"file_id":0,"rel_path":"Photos/img 01.jpg","size":123456,"modified_ms":1762500000000}
```

`rel_path` uses `/` only, must not be absolute, and may not contain `..`, empty components, backslashes or control characters. The receiver additionally applies platform-hardening rules (reserved DOS names, Windows-illegal characters, trailing dots/spaces, 200-character components).

## Resume entries

`TRANSFER_ACCEPT.entries` tells the sender what the receiver already has:

```json
{"file_id":0,"offset":4194304,"prefix_hash":"…","completed":false}
```

- `prefix_hash` is BLAKE3 of exactly `offset` bytes on disk (or of the whole file when `completed` is true).
- The sender hashes the same prefix of the source. If it differs, the sender restarts that file at offset 0 (`FILE_START` with offset 0 truncates the partial file).
- `completed=true` means the receiver finalized and verified this file in an earlier session, and revalidated it (size + full hash) when opening the new session. The sender still hashes its source and skips the file only on an exact match.
- An incomplete file is never offered with `offset == size`; such a file restarts so final verification cannot be skipped.

## Session sequences

### Trusted returning device

```
TLS 1.3 handshake (mutual certs)
C→S HELLO                       {protocol:1, name, client_nonce}
S→C HELLO_ACK                   {client_trusted:true, pairing_available, server_nonce, server_fp}
S→C SESSION_READY               {session_id}
C→S TRANSFER_REQUEST
S→C TRANSFER_ACCEPT | TRANSFER_REJECT
   (user approval on the receiver precedes ACCEPT)
C→S FILE_START
C→S FILE_CHUNK (control) + data frame     … repeated
C→S FILE_COMPLETE
S→C FILE_ACK {verified:true}
   … repeated per file
C→S TRANSFER_COMPLETE
S→C TRANSFER_ACK
```

The client aborts if the receiver's declared fingerprint does not equal the certificate fingerprint captured from TLS. The client also aborts when the receiver claims to trust the client but the client has no pin for the receiver (e.g. after a reinstall): proceeding would trust an unverified server. A fresh pairing is required in that case.

### First-time pairing

```
TLS 1.3 handshake (mutual certs)
C→S HELLO
S→C HELLO_ACK {client_trusted:false, pairing_available:true, pairing_expires_secs:300}
C→S PAIR_REQUEST {device_name, fingerprint, proof}
   receiver verifies proof → shows device name + fingerprint → user approves
S→C PAIR_ACCEPT {fingerprint}
S→C SESSION_READY
   then TRANSFER_REQUEST…
```

`proof = keyed_blake3(derive_key("shadow-transfer pairing v1", code), "SHADOW-PAIR-v1" || client_fp || server_fp || client_nonce || server_nonce)`.

The code is 8 characters from a 31-symbol unambiguous alphabet (no O/0, I/1, L). It expires after 300 seconds, is single-use, and is rotated after a successful pairing. Failed proofs are delayed by 750 ms; more than five failures in 60 seconds disables pairing until the window passes. A rejection closes that connection.

### Cancellation

Either side may send `CANCEL`. The sender also aborts reliably if the user presses Cancel; partial receiver state is retained for a later resume. The receiver can cancel an active transfer through its own handle; it then sends an `ERROR` with code `409` so the sender does not silently retry.

### Errors

| Code | Meaning |
|---|---|
| 400 | generic protocol/validation/transfer failure |
| 409 | transfer cancelled by receiver |

On any error the connection is closed after the error frame when possible. Partial files are kept for resume unless the transfer completed.

## Limits summary

| Item | Limit |
|---|---|
| Control frame | 256 KiB |
| Data frame | 8 MiB |
| Files per transfer | 100,000 |
| Relative path | 4096 bytes wire, 200 chars per component |
| File size | 2^50 bytes |
| Concurrent receiver connections | 4 |
| Handshake | 15 s |
| Per-message I/O | 60 s (600 s while the receiver hashes a large file) |
| Transfer approval | 180 s |
| Pairing response | 180 s (approval on receiver) |

## Chunking

The sender starts at 1 MiB and adapts between 256 KiB and 4 MiB based on observed throughput. Chunk boundaries are arbitrary; the receiver only requires contiguous offsets. Benchmarks on loopback showed no consistent benefit above 1 MiB, so the default stays conservative for Wi-Fi and the tablet.

## Versioning

- Frame version and application protocol version are separate. Both are currently 1.
- `HELLO` carries the application version; a mismatch is rejected before pairing.
- New optional fields must not be added to v1 payloads because unknown fields are rejected. A future version must negotiate a new protocol version and may keep the framing.
- QUIC, if added later, must reuse the same control message semantics and resume rules; only the stream transport changes.

## Wire example

```
00 00 00 00  53 54 58 31 01 00 00 00 72 00 00 00   STX1 v1 control len=114
7b 22 74 79 70 65 22 3a 22 50 49 4e 47 22 2c ...   {"type":"PING","payload":{...}}
```
