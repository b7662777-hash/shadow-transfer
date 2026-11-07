# Windows application

Executable: `ShadowTransfer.exe` (release and portable).
Installer: `dist/ShadowTransfer-Setup.exe` (Inno Setup 6, per-user install, unsigned).

## Features

- Dark, minimal egui UI with the Shadow Transfer branding.
- Device list from LAN discovery (one 3-second scan at launch, plus manual **Scan**), trust badges, manual IP connection.
- Drag-and-drop files and folders anywhere on the window; native file/folder pickers.
- Separate **AUTO DISCOVERY** and **MANUAL CONNECTION** cards, each with its own pairing-code field. The port field accepts digits only.
- A standalone **Pair** action: pairing no longer requires choosing files or starting a transfer.
- Send dialog with the same optional code; first use requires the code shown on the receiver and approval there.
- Receive mode toggle, pairing code with countdown and regeneration, chosen download folder, firewall hint.
- Incoming pairing and transfer approval dialogs with device name, fingerprint, file preview and total size.
- Queue with per-transfer direction, peer, file name, bytes, rolling speed, average speed, ETA, phase, BLAKE3 status.
- Cancel for outgoing and incoming transfers; Retry for failed outgoing transfers (reuses the transfer id, enabling resume).
- History persisted to JSON (last 200 entries).
- Settings: device name, receive port, download folder, identity fingerprint, trusted devices with revoke.
- No animations, no telemetry, no auto-update.

## Data locations

```
%LOCALAPPDATA%\ShadowTransfer\
├── identity.cert.der     self-signed device certificate
├── identity.key.der      private key (user-profile ACL; not DPAPI-wrapped)
├── trusted.json          pinned peer fingerprints
├── settings.json         device name, download folder, receive port
├── history.json          recent transfers
└── shadow.log            rotating diagnostic log (see below)
```

Default receive folder: `%USERPROFILE%\Downloads\ShadowTransfer` unless changed.

## Receive flow

1. Header toggle → **Receive**.
2. The app binds TCP `0.0.0.0:<port>` (default 47831), starts a 1 Hz UDP beacon and shows a pairing code valid for five minutes.
3. A trusted peer connects directly. A new peer must send a valid code proof; the app shows an approval dialog with its name and certificate fingerprint.
4. Each incoming transfer also requires approval.
5. Files are written to `<receive folder>\.shadow-transfer-state\<transfer-id>\`, verified, then placed in the receive folder preserving relative paths. Existing names get ` (1)`, ` (2)` suffixes; nothing is overwritten.
6. Turning receive off stops the listener, beacon, and pending approvals immediately.

## Send flow

1. Wait for the launch scan or press **Scan**; the tablet appears with a **code needed** or **trusted** badge.
2. Click the tablet, paste the code from its **Receive** screen into the card's pairing-code field, and press **Pair with this device**.
3. On the tablet, tap **Approve** for the pairing request. Keep the app open. The Windows card shows "Waiting for approval on the tablet" until this happens.
4. Once the badge says **trusted**, select files/folders (drag-drop or buttons).
5. Press **Send**, review the confirmation, press **Send**, then tap **Approve** on the tablet for the incoming transfer.
6. Progress is measured, not simulated. A file is marked verified only after the receiver confirms its own BLAKE3 re-hash.
7. A failed send can be retried from the card; the same transfer id lets the receiver resume from its verified prefix.

**Manual connection** is a separate card, not mixed with auto discovery: enter the tablet's Wi-Fi IP, port `47831`, its own pairing code, and press **Connect & pair**. The code never goes in the port field. After pairing, the same file/send flow applies.

## Logging

Release builds have no console. The app writes INFO/WARN diagnostics to a daily rotating file:

```
%LOCALAPPDATA%\ShadowTransfer\shadow.log.YYYY-MM-DD
```

Pairing codes and file contents are never logged. Reproduce a problem, then attach the relevant log plus the receiver's log if available.

## Debug/test hooks

These environment variables exist for UI smoke tests. Auto-approve and autosend are **disabled in release builds** (`debug_assertions`); they must never be used outside a test.

| Variable | Effect |
|---|---|
| `SHADOW_UI_TOPMOST=1` | Window stays above other windows (screenshot tests) |
| `SHADOW_UI_AUTOSTART_RECEIVE=1` | Start receive mode at launch |
| `SHADOW_UI_AUTO_APPROVE=1` | Debug only: auto-approve pairing/transfer requests |
| `SHADOW_UI_DEMO_FILES=<path>` | Debug only: stage a file/folder at launch |
| `SHADOW_UI_AUTOSEND_PEER=ip:port` | Debug only, with `SHADOW_UI_AUTOSEND_CODE=<code>`: send staged files automatically |

The normal button-driven flow uses the same code paths; hooks only remove mouse input from automated tests.

## Installer

`apps/windows/installer/ShadowTransfer.iss` builds with Inno Setup 6.7:

```
"C:\Users\<you>\AppData\Local\Programs\Inno Setup 6\ISCC.exe" apps\windows\installer\ShadowTransfer.iss
```

Output: `dist/ShadowTransfer-Setup.exe`. It is a per-user install (`PrivilegesRequired=lowest`), creates Start Menu/optional desktop shortcuts, includes the license and README, and registers an uninstaller. The binary and installer are **unsigned**, so Windows SmartScreen may warn; this is expected for a local build.

## Known gaps

- Android→Windows was not exercised on the tablet; Windows→Android was validated on a real SM-T505 (449 MB and 1.845 GB verified transfers).
- Windows key storage is user-profile ACL, not DPAPI; see SECURITY.md.
- Only IPv4 LAN is supported.
