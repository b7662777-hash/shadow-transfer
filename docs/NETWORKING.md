# Networking

## Ports

| Port | Protocol | Purpose | Active when |
|---|---|---|---|
| 47831 | TCP | TLS 1.3 transfer listener | Receive mode enabled (Windows port configurable; Android fixed) |
| 47830 | UDP | Discovery beacons / discovery window | While receiving: one beacon per second. While scanning: a 3-second receive window |
| ephemeral | TCP | Outgoing transfer connection | During a send only |

No other sockets are opened. With receive mode off, Windows opens no listener and performs no discovery; Android stops the Rust receiver and foreground service. There is no cloud component, relay, account service or telemetry endpoint.

## Discovery

### Beacon

UDP broadcast to port 47830. Payload ≤1024 bytes, JSON:

```json
{
  "magic": "SHADOW-TRANSFER-BEACON/1",
  "name": "Workstation",
  "port": 47831,
  "fingerprint": "…64 hex chars…",
  "pairing": true
}
```

Validation requires the exact magic, a valid device name, a non-zero port and a 32-byte hex fingerprint. Unknown fields are rejected.

### Sending side lifecycle

- A short explicit scan (`discover_for`, 3 seconds) binds UDP 47830 with address reuse and collects the newest beacon per fingerprint.
- The sender derives the device address from the datagram source IP plus the advertised TCP port, so it works across multiple interfaces.
- Scanning stops at the end of the window. There is no background scan and no polling loop.
- The Windows app performs short discovery windows continuously while the window is focused (currently every few seconds), and immediately on **Scan**. The Android app runs foreground-only discovery automatically while the app is in the foreground, with a manual **Scan** action. Both stop when the app leaves the foreground/minimizes; there is no background scanning.
- Devices are shown with a trust badge (Windows) based on the trust store. "code needed" means the device is discovered but not paired. Names and fingerprints are unauthenticated at this stage.

### Receiving side lifecycle

- Enabling receive mode starts the TCP listener, then broadcasts a beacon every second.
- Disabling receive mode cancels advertising immediately and stops the listener (and the Android foreground service).
- Beacons are sent to `255.255.255.255`, loopback and each interface's computed broadcast address; send failures on one interface are logged and ignored.

### Why UDP broadcast instead of mDNS/DNS-SD

mDNS is a good fit for general service discovery but adds platform asymmetry: Android NSD, Windows mDNS stacks, multicast permissions, daemon behavior and firewall rules differ. For exactly two device classes on a home LAN, a 100-byte beacon plus manual IP gives the same result with far less code and no daemon. If mDNS is added later, it must be an additional discovery source, not a change to the transfer protocol.

## Manual connection

Manual IP + port always works and is the fallback when a router blocks broadcast, when the AP isolates clients, or when discovery is disabled. The Windows UI accepts `IP` plus a port field or `IP:port`; the Android UI accepts the IP and a port field (the discovered address fills both).

Discovery is a convenience, not a trust mechanism. Pairing must still succeed.

## Firewall

Windows Defender Firewall prompts on the first listener/beacon activity. Allow Shadow Transfer for **private** networks. If the prompt was dismissed, add an inbound rule for `ShadowTransfer.exe` (private profile). A blocked firewall produces: discovery works from another device but connection attempts time out; or no device is seen at all.

## Android networking specifics

Permissions used:

- `INTERNET`, `ACCESS_NETWORK_STATE`, `ACCESS_WIFI_STATE`
- `CHANGE_WIFI_MULTICAST_STATE` — a `MulticastLock` is held **only** during the explicit discovery window so broadcast beacons are not filtered while the screen is on.
- `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC` — receiver runs in a `dataSync` foreground service with a low-importance notification.
- `POST_NOTIFICATIONS` — requested at runtime on Android 13+.
- `ACCESS_LOCAL_NETWORK` — declared for Android 17 / API 37+ forward compatibility and requested at runtime when the SDK level is ≥37. The current build targets API 34, where it is not enforced.

No Wi-Fi lock, wake lock, or unrestricted storage permission is used. The tablet may enter Doze when the app is in the background; the foreground service is intended to keep transfers reliable while receive mode is enabled. Behavior under vendor battery optimization must be validated on the physical tablet — this was not possible in this build.

## Timeouts and recovery

| Operation | Timeout |
|---|---|
| TCP connect | 5 s |
| TLS handshake | 15 s |
| Application handshake message | 15 s |
| Regular message / chunk data | 60 s |
| Receiver approval of pairing/transfer | 180 s (auto-decline on timeout) |
| Full-file verification acknowledgement | 600 s |
| File collection / walk | bounded by filesystem; no artificial timeout |

The sender makes at most two attempts per send. A retry reconnects, repeats the handshake (trust is normally already established), and sends the same `transfer_id`, so the receiver returns verified resume offsets. Cancellation by the user stops retries.

## Network constraints

- IPv4 LAN only: the receiver binds `0.0.0.0`; discovery uses IPv4 broadcast. IPv6 transfer is not implemented.
- Both devices must be able to open a TCP connection to each other. Guest Wi-Fi / AP isolation blocks this even though both are "on the same network".
- A VPN on either device usually breaks direct LAN reachability; disable it for transfers.
- 2.4 GHz and 5 GHz bands on the same router are normally bridged, but some routers isolate them.
- No UPnP, NAT-PMP, port forwarding or cloud relay is used or configured.

## Privacy

Device names and certificate fingerprints are broadcast on the LAN while receive mode is on. File data and names are visible only inside the TLS session. There is no analytics, update check or external connection.
