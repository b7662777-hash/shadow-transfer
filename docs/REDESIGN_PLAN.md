# Shadow Transfer v0.2 Product Redesign Plan

Status: M1 partially implemented on `redesign/v0.2` (Windows continuous discovery/last-seen/trusted selection; Android foreground auto-discovery and trusted-device list/revoke). QR and platform metadata remain in M1.
Baseline: v0.1.0 (`main`, release published). The working transfer engine and protocol are preserved.

## 1. What exists today

| Area | Current implementation | Gap vs. the redesign brief |
|---|---|---|
| Transfer engine | Rust streaming, TLS 1.3, pinning, BLAKE3, resume, cancel, folder support | No parallel-stream/QUIC path; pause not supported |
| Discovery | UDP beacon while receiving; one scan at Windows launch; Android scan on demand | No continuous foreground discovery, online/offline state, platform identification |
| Pairing | Standalone code pairing, explicit approval, auto/manual cards | No QR; no trusted-device quick connect |
| Trusted devices | Windows list/revoke; Rust trust store | Android has no list/revoke UI; no auto-connect policy |
| Windows UI | Single screen: devices/files/transfers/history/settings modal | No navigation shell, Home dashboard, activity filters, diagnostics |
| Android UI | Single scrollable screen with cards | No bottom navigation, Home/Activity/Settings separation, trusted devices |
| Settings | Windows basic settings; Android device name | No Basic/Advanced split, network diagnostics, configurable transfer/network options |
| Notifications | Android static foreground notification | No progress/completion/approval notifications; no Windows toast integration |
| History | Windows persisted; Android last 50 in memory/file | No tabs/search/open file/open folder/retry/share |
| File collisions | Automatic unique suffix | No keep both/replace/skip/rename choice |
| Incoming policy | Always ask | No trusted-device auto-accept policy |
| Storage | SAF staging + import; app-private Rust keys | No Keystore/DPAPI wrapping; no direct SAF streaming |
| Diagnostics | Logs only | No network/port/firewall test UI, no log export |

## 2. Design direction

- Unique Shadow Transfer identity: deep navy/charcoal surfaces, blue-to-cyan accent, clean white/grey typography, rounded 16–20 px cards, subtle 1 px borders, restrained shadows.
- Dark theme default; optional light theme.
- No technical details (IP, port, fingerprints) on the Home screen; they live in Advanced/Diagnostics.
- Accessibility: text ≥ 14 sp, contrast AA, touch targets ≥ 48 dp, no color-only status.
- Lightweight animation only for progress transitions and screen changes.
- Consistent icon set and wording across Windows and Android.

## 3. Architecture additions

Keep the Rust engine and protocol. Add thin layers above it:

```
UI (Compose / egui)
  ↓
App state (repositories, navigation, settings)
  ↓
Connection Manager
  ├── Network state / diagnostics
  ├── Discovery coordinator (continuous, lifecycle-aware)
  ├── Trusted-device / recent-device store
  └── Connection providers: LAN, manual, hotspot guidance, Wi-Fi Direct (platform-gated)
  ↓
Existing Rust engine (TLS 1.3, protocol, BLAKE3, resume)
```

- Discovery coordinator: foreground periodic windows, device registry with `online/offline/last_seen`, platform metadata via a versioned beacon extension that preserves v1 compatibility.
- Trusted/recent registry: Rust trust store remains the source of truth; recent devices persisted in app config.
- Diagnostics service: interface/IP/port checks, discovery/TCP tests, log export.
- Settings model: explicit, typed options; only expose settings that are actually wired.

## 4. Milestones

### M1 — Effortless discovery and trusted connections

Deliverables:

- Continuous discovery while the app is in the foreground; no repeated Scan presses.
- Device registry with online/offline/last-seen and platform where available.
- Recent devices; auto-select a trusted device when it appears.
- Android trusted-device management: list, fingerprint, revoke.
- QR pairing payload + display on receiver; Android scanning; Windows display. Windows camera scanning is investigated separately (no native camera stack today).
- Versioned discovery beacon to add platform metadata without breaking v0.1 peers.
- Windows/Android UI feedback for discovery failures (firewall, isolation, VPN).

Acceptance:

- Discovery updates automatically within 10 seconds of a tablet enabling Receive.
- A device that disappears is marked offline, then removed per policy.
- Revoking a trusted device on Android requires pairing again.
- QR payload is short-lived and contains no long-term secret.
- Existing E2E tests and pairing behavior remain green.

### M2 — Premium design system and navigation shells

Deliverables:

- Shared design tokens (dark + light) for Windows and Android.
- Windows: left navigation rail (Home, Send, Receive, Activity, Trusted Devices, Settings, Advanced).
- Android: bottom navigation (Home, Activity, Settings) with Home dashboard:
  connection card, SEND/RECEIVE actions, incoming transfers, recent transfers.
- Responsive tablet/desktop layouts; empty states; accessible typography.

Acceptance:

- No technical fields on Home.
- Both platforms use the same accent/status semantics.
- Existing send/receive flows remain reachable within two taps/clicks.

### M3 — Transfer UX, approvals, notifications, history

Deliverables:

- Incoming policies: ask always; auto-accept from individually trusted devices (explicit opt-in).
- Collision policy: keep both, replace with confirmation, skip, automatic rename.
- Transfer detail view: file list, per-file progress, speed, ETA, failures, cancel/retry.
- Notifications: incoming request, started, completed, failed, connection lost. Android foreground service + channels; Windows toast integration if the runtime supports it without a heavyweight dependency.
- Activity tabs/search/open file/open folder/retry/share; deleting history never deletes files.

Acceptance:

- Unknown devices can never auto-accept.
- No file is overwritten without an explicit policy.
- Notifications and history remain consistent after restart.

### M4 — Settings and diagnostics

Deliverables:

- Basic settings with short explanations: device name, theme, language, download folder, notifications, auto-discovery, auto-connect, receive policy, startup behavior.
- Advanced sections: Network info; TCP; UDP; IP; Transfer; Security; Diagnostics. Only real, wired options.
- Restore defaults; warnings for disruptive changes.
- Diagnostics: interface/IP/port checks, discovery test, TCP connect test, firewall guidance, log export.
- Android trusted-device page and certificate fingerprint.

Acceptance:

- Changing a setting affects the real engine or is not shown.
- Restore defaults returns a known-good state.
- Diagnostics explanation matches the observed failure.

### M5 — Platform connection modes and key hardening

Deliverables/decisions:

- Hotspot: Android public APIs generally cannot enable a hotspot programmatically; implement guided system-settings flow and detect when the peer appears. Do not claim one-tap hotspot.
- Wi-Fi Direct: investigate Android Wi-Fi P2P APIs, implement only if it can be tested on the physical tablet and does not break LAN flow; Windows does not receive a fake Wi-Fi Direct option.
- IPv6 and mDNS: evaluate as additional discovery/transport paths; keep UDP + manual as fallback.
- Android Keystore / Windows DPAPI: wrap identity keys using platform APIs; preserve identity migration or document a re-pair requirement.

Acceptance:

- Unsupported modes are explained, not faked.
- Key wrapping is tested (including failure to unwrap).
- No silent downgrade or plaintext path.

### M6 — Validation and v0.2 release

- Windows ↔ Android, Android ↔ Android, Windows ↔ Windows.
- 1 MB to 5 GB, folders, thousands of files, cancellation, resume, network loss, low storage, screen-off.
- Wi-Fi vs raw throughput, memory, battery, cold/warm start.
- Regression suite, CI green, release APK/EXE/installer and release notes.

## 5. Feasibility and constraints

| Feature | Decision |
|---|---|
| Continuous LAN discovery | Feasible; lifecycle-aware foreground scans, battery policy documented |
| Platform metadata | Feasible via a second, versioned beacon; v1 beacon retained for compatibility |
| QR pairing | Feasible: receiver displays QR with IP/port/short-lived code; Android scans. Windows camera scanning requires a camera stack and is not assumed |
| Automatic hotspot control | Not generally available to ordinary Android apps; guided setup only |
| Wi-Fi Direct | Android-only and device-dependent; implement only after physical testing, never advertise on Windows |
| IPv6 / mDNS | Feasible additions, but no promise until tested on the target router/tablet |
| Android Keystore / Windows DPAPI | Feasible but requires migration and platform-specific signing callbacks; milestone 5 |
| Pause/resume of an in-flight transfer | Protocol work; resume after interruption exists. Full pause is deferred |
| Direct SAF streaming | Would remove the cache copy but requires fd-based receiving and complex resume; evaluate after core UX |
| Clipboard/text/sharesheet | Secondary; only after the core transfer UX is solid |

## 6. Missing input

The uploaded brief references a DroidMac screenshot, but only the text brief was attached. The design will follow the specified visual language (navy/charcoal, blue-cyan, rounded cards) and a unique ShadowTransfer identity. If the screenshot is required as a closer reference, attach the image.

## 7. Branch and release strategy

- `main` remains the stable v0.1.0 line.
- Redesign work is developed on `redesign/v0.2` with focused commits and PRs.
- v0.1.0 artifacts remain available.
- No release claims are made for a feature until it is implemented, wired, tested, and documented.

## 8. First implementation step

M1: lifecycle-aware continuous discovery, device registry (online/offline/last seen), trusted-device quick connect, Android trusted-device list/revoke, and the discovery/QR design spike. Then M2 visual shells.
