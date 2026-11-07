# Android application

Package: `com.shadowtransfer.app`
Min SDK: 26 · Target/compile SDK: 34 · ABIs packaged: `arm64-v8a`, `armeabi-v7a`, `x86_64`
UI: Kotlin + Jetpack Compose (Material 3, AMOLED-friendly dark theme)

## Build status

- Debug APK: `apps/android/app/build/outputs/apk/debug/app-debug.apk`
- Release APK: `apps/android/app/build/outputs/apk/release/app-release.apk` (~8.8 MiB, R8 minified, signed with the local `apps/android/release.keystore`)
- Rust JNI libraries: `apps/android/app/src/main/jniLibs/<abi>/libshadow_android.so`
- **Validated on a real Samsung Galaxy Tab A7 (SM-T505):** paired from Windows over the LAN and completed real transfers of 449 MB and 1.845 GB, both acknowledged as verified by the Android receiver. Real-world throughput on that 5 GHz link was about 15 MB/s; the latest TCP-buffer, incremental-verification and Wi-Fi-lock changes require a re-test with the updated APK. Battery drain and long unattended/background runs were not measured.

## Architecture

```
MainActivity (Compose)
   └── ShadowRepository (application singleton)
         ├── StateFlow<UiState> for the UI
         ├── polls ShadowEngine.nativePollEvent() every 120 ms (IO dispatcher)
         ├── SAF staging and import
         ├── history persistence (50 entries)
         └── TransferService (foreground, dataSync)
   └── ShadowEngine (JNI)
         └── Rust engine: identity, trust, TLS, transfer, discovery
```

The Rust engine is the same crate used by Windows (`shadow-transfer`, `shadow-crypto`, `shadow-protocol`, `shadow-discovery`). Kotlin does not implement any transfer, protocol, pairing or hashing logic.

## Storage

### Receiving (SAF)

1. The user selects a destination tree via the system picker. The app calls `takePersistableUriPermission` and stores the URI.
2. Rust receives into app staging: `getExternalFilesDir(null)/incoming/...`.
3. Every file is verified (chunk hashes and full BLAKE3) before `file_received` is emitted.
4. Kotlin copies the verified file from staging into the chosen document tree, recreating subdirectories and avoiding name collisions. The staging copy is deleted after a successful copy.
5. If the provider fails or permission is lost, the verified file is kept in staging and an error is shown; data is not deleted.

When no destination has been selected, files remain in app staging and the UI reports the path.

### Sending (SAF → cache staging)

SAF URIs are not filesystem paths, so selected documents/folders are copied into `cacheDir/outgoing/batch-<timestamp>/` first. Rust then reads those real files. This doubles disk I/O for sends; it is the cost of not requesting broad storage permissions. The staging batch is deleted after a successful send.

## Lifecycle

- Receive mode starts a `dataSync` foreground service with a low-importance notification so transfers can continue while the app is not in the foreground. Android 14+ requires `FOREGROUND_SERVICE_DATA_SYNC`, which is declared.
- Turning receive off stops the service and the Rust listener/advertiser.
- No permanent wake or Wi-Fi lock is used. A `MulticastLock` is held only during foreground discovery windows (automatic while the app is foreground, or manual) so broadcast beacons are received.
- The trusted-device list is available in Settings; revoking a device removes its pin and requires pairing again. During an active transfer, a `WIFI_MODE_FULL_HIGH_PERF` Wi-Fi lock is held to avoid power-save latency spikes and is released when no transfer is active.
- Doze and vendor battery management may still affect long background receives; the foreground service is the intended mitigation. This was not validated on the tablet.

## Permissions

| Permission | Why |
|---|---|
| `INTERNET`, `ACCESS_NETWORK_STATE` | TCP/TLS transfer |
| `ACCESS_WIFI_STATE`, `CHANGE_WIFI_MULTICAST_STATE` | discovery window multicast lock |
| `CHANGE_WIFI_STATE` | transfer-only high-performance Wi-Fi lock |
| `POST_NOTIFICATIONS` | foreground service notification (runtime prompt, Android 13+) |
| `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC` | receive/transfer service |
| `ACCESS_LOCAL_NETWORK` | declared for Android 17/API 37+; runtime requested when SDK ≥37 |

No storage permission is requested. SAF grants access per chosen folder.

## Pairing and approvals

- Receive screen shows the current 8-character code and countdown; **New code** rotates it.
- Incoming pairing requests appear as an AlertDialog with the device name and certificate fingerprint; **Approve** or **Decline** answers the Rust receiver.
- Incoming transfers require a second approval with file count, total size and a preview.
- Trust is persistent in Rust's trust store. There is currently **no trusted-device list or revoke UI on Android**; to fully reset trust, clear app data (this also rotates identity) or revoke from the Windows side where available. This is a known gap.

## Sending from Android

1. Choose files or a folder (copied to cache staging).
2. Optionally press **Scan** to discover receivers (MulticastLock is acquired for the 3-second window), or type the receiver IP and port (47831).
3. Enter the receiver's pairing code on first use.
4. Send. Progress, speed, ETA, cancel and BLAKE3 verification are shown.
5. Failed sends are not automatically retried and do not expose the resume id in the UI; start a new send to retry. The engine's resume support is used for automatic retries within one send, not across UI attempts.

## Receive folder behavior

- Folder structure is preserved via `DocumentsContract` directory creation.
- Duplicate names get ` (1)`, ` (2)` suffixes.
- Insufficient storage surfaces as a write error; the transfer is marked failed, partial data is retained for a possible resume, and the UI reports the error. The SAF provider is not modified until a file is fully verified.

## Installation

```
adb install -r apps/android/app/build/outputs/apk/release/app-release.apk
```

For local debugging, use the `.debug` application id:

```
adb install -r apps/android/app/build/outputs/apk/debug/app-debug.apk
```

Release signing uses a locally generated keystore. This is not a Play Store-ready signing setup; see BUILD.md.

## Not implemented on Android

- Android Keystore integration for the identity key (Rust stores it in app-private storage).
- QR-code display (secure pairing code is supported; QR is optional in the requirements).
- Trusted-device management UI.
- Explicit manual retry with the same transfer id (automatic in-send retry/resume works).
- mDNS discovery (UDP broadcast is shared with Windows).
