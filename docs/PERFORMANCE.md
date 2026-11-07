# Performance

All numbers below are measured, not estimated. They are **loopback** results on the development PC, not Wi-Fi or tablet numbers.

## Environment

- Windows 11 (10.0.26200), Intel Core i5-1235U, 24 GB DDR4, NVMe SSD.
- Sender and receiver both on 127.0.0.1 in one process, release build.
- Every result includes TLS 1.3, framing, disk write, per-chunk BLAKE3 and the receiver's full-file re-hash before acknowledgement.
- Files were created from an 8 MiB repeated pattern; BLAKE3 speed is content-independent. OS page cache may help repeat reads; 5 GiB results are more disk-bound.

## Chunk size sweep (100 MiB file)

Three runs, same machine; range shows run-to-run variance (NTFS, cache, CPU scheduling):

| Chunk | Throughput (MiB/s) |
|---|---|
| 256 KiB | 272.9 – 326.3 |
| 512 KiB | 286.7 – 347.3 |
| 1 MiB | 294.8 – 354.0 |
| 2 MiB | 246.3 – 376.2 |
| 4 MiB | 264.6 – 280.7 |
| Adaptive (256 KiB–4 MiB) | 200.1 – 366.0 |

There is no consistent gain above 1 MiB, and 4 MiB was never fastest. The engine therefore keeps a 1 MiB starting point with bounded adaptation. Larger chunks increase worst-case latency and memory without a measured throughput benefit on this link.

## File-size sweep (1 MiB chunks)

| Size | Time | Throughput |
|---|---|---|
| 1 MiB | 28.2 ms | 35.5 MiB/s |
| 10 MiB | 67.2 ms | 148.7 MiB/s |
| 50 MiB | 179.4 ms | 278.8 MiB/s |
| 100 MiB | 284.8 ms | 351.1 MiB/s |
| 500 MiB | 1411.8 ms | 354.2 MiB/s |
| 1 GiB | 2726.5 ms | 375.6 MiB/s |
| **5 GiB** | **22.38 s** | **228.8 MiB/s** |

Aggregate for the six-size sweep: **1685 MiB in 4.70 s = 358.6 MiB/s**.

The 5 GiB case is slower because source creation/read, receiver writes and the verification re-read all hit the disk instead of cache. Sustained throughput still exceeds the 200 MB/s (≈190 MiB/s) floor mentioned in the requirements, on loopback.

## Comparison with raw network

| Path | Throughput |
|---|---|
| Raw TCP loopback, 1 GiB, no TLS/framing/hash/disk | **1294 MiB/s** |
| Shadow Transfer, 1 GiB | 375.6 MiB/s |
| Shadow Transfer / raw TCP | ~29% |

The gap is expected: TLS encryption, frame copies, writing the destination file, hashing outbound data and re-reading/hashing the delivered file. It is not attributed to one component here; no optimization was applied without measurement.

## Component microbenchmarks (Criterion)

| Benchmark | Result |
|---|---|
| BLAKE3, 4 MiB buffer | 948.9–961.2 µs for 4 MiB → **4.06–4.12 GiB/s** |
| Decode `FILE_CHUNK` control JSON | **≈1.03 µs** |
| Decode `HELLO` control JSON | **≈0.79 µs** |
| Encode `PING` | sub-microsecond |

BLAKE3 is roughly ten times faster than the observed transfer rate, so hashing is not the bottleneck at LAN speeds. Control-message parsing is negligible compared with a 1 MiB payload.

## Discovery and pairing observations

- Loopback discovery test: an advertiser sending every 200 ms is found inside the 3 s discovery window. Production beacons are every 1 s; the discovery UI waits up to 3 s.
- In the Windows GUI test, a 12 MiB transfer to a local receiver completed in 95 ms including pairing setup on a first connection. This is a local observation, not a Wi-Fi latency benchmark.
- Pairing itself is dominated by human approval on the receiver; protocol work is sub-second locally.

## Requirements vs reality

| Requirement | Status |
|---|---|
| Discovery <2 s on a healthy LAN | Not measured on real Wi-Fi; advertising interval is 1 s |
| Pairing <3 s after discovery | Protocol path is sub-second locally; human approval and Wi-Fi unmeasured |
| Connection <1 s | Local yes; Wi-Fi unmeasured |
| Small files <5 s | 1 MiB took 28 ms loopback; Wi-Fi unmeasured |
| Maximum sustainable large-file throughput | 324.9 MiB/s for 5 GiB loopback after tuning (228.8 before); Wi-Fi untested |
| Near-zero idle CPU / no idle traffic | Receive-off mode opens no sockets and no discovery. Measured with the release UI open and receive off: ~0.3–0.6% of one core over 5 s (15.6–31.3 ms CPU) |
| Bounded memory | Design: 8 MiB frame cap, ≤4 MiB send buffer, 256 KiB hash buffers. Measured Windows release process working set while idle with the window open: ~203 MiB (egui/GPU context plus CJK font fallback). Memory during transfers was not separately instrumented. |
| Every transfer verified | Enforced by protocol and tests |

## Final speed tuning

Three changes were made after the first real Wi-Fi measurement (585 Mbps PHY, ~15 MB/s actual, latency spikes up to 279 ms on a 5 GHz link):

1. **8 MiB TCP send/receive buffers on both peers.** A single stream over a high-latency Wi-Fi link can be limited by the TCP window rather than the PHY rate. These are best-effort `SO_SNDBUF`/`SO_RCVBUF` requests; the OS may clamp them.
2. **Incremental receiver verification for fresh files.** The receiver already BLAKE3-hashes every incoming chunk. For a file started at offset 0 it now finalizes that incremental hash instead of re-reading the complete file from disk before acknowledging. Resumed files still perform a full re-read, and sender/receiver hashes must still match. This removes a second 1.7 GB disk read on the tablet and shortens completion time; it does not increase network MB/s.
3. **Android Wi-Fi high-performance lock while a transfer is active** (`WIFI_MODE_FULL_HIGH_PERF`), released when no transfer is active. This targets Wi-Fi power-save latency spikes; it is not held while idle.

Post-tuning loopback measurements (same machine, one run; loopback remains noisy):

| Case | Before tuning | After tuning |
|---|---|---|
| 100 MiB, best chunk | 376.2 MiB/s | **490.1 MiB/s** |
| 100 MiB, adaptive | 331.8–366.0 MiB/s | **459.2 MiB/s** |
| 1 GiB | 375.6 MiB/s | 284.0 MiB/s |
| 5 GiB | 228.8 MiB/s | **324.9 MiB/s** |
| Raw TCP loopback (no tuned path) | 729–1294 MiB/s | 1088 MiB/s |

The 5 GiB gain is consistent with removing the receiver's second full-file read; the 1 GiB run is within the observed noise range. Loopback numbers do not predict Wi-Fi, and no tablet speed re-test was possible here. The Wi-Fi lock and larger socket windows matter only on the real link.

### Realistic expectations

- 1.7 GB in "seconds" requires >1.4 Gbps end-to-end. A Galaxy Tab A7-class Wi-Fi 5 client will not achieve that on a normal access point.
- At the observed ~15 MB/s, 1.7 GB takes about 2 minutes. A healthy 5 GHz link at 30–60 MB/s would take ~30–60 seconds.
- The physical link, Wi-Fi power saving, airtime contention, tablet storage and SAF import are outside the Rust transfer engine's control.

## Not measured

- Wi-Fi throughput between the PC and the Galaxy Tab A7.
- Tablet CPU, RAM and battery consumption.
- JNI/serialization overhead on Android.
- SAF import cost on the tablet.
- Cold/warm app start times.
- Idle RSS on Android and memory during transfers on either platform. The Windows idle working set (~203 MiB) was measured with PowerShell; it is not representative of the tablet.
- Post-tuning Wi-Fi throughput on the tablet. The TCP-buffer and Wi-Fi-lock changes require the updated APK and a new measurement on real hardware.

## How to reproduce

```
cargo run --release -p shadow-transfer --example bench_transfer -- "1,10,50,100,500,1024" 1024
cargo run --release -p shadow-transfer --example bench_transfer -- "5120" 1024
cargo bench -p shadow-transfer --bench core
powershell -File scripts/run-benchmarks.ps1
```

Raw measured outputs are preserved in:

- [benchmarks/loopback-1g.txt](benchmarks/loopback-1g.txt)
- [benchmarks/loopback-5g.txt](benchmarks/loopback-5g.txt)
- [benchmarks/loopback-raw-tcp.txt](benchmarks/loopback-raw-tcp.txt)

Results depend heavily on storage and background load; compare runs on the same machine and do not compare loopback numbers with Wi-Fi expectations.
