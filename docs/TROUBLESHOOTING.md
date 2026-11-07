# Troubleshooting

## Where do I enter the pairing code?

Pairing is a separate step now, in two clearly separated cards:
1. On the tablet, make sure **Receive** is on and read the 8-character code (create a new code if it expired).
2. **AUTO DISCOVERY:** click the discovered tablet, paste the code into that card's field, and press **Pair with this device**.
3. **MANUAL CONNECTION** (only if discovery is blocked): enter the tablet IP, port `47831`, its own code field, and press **Connect & pair**.
4. **Tap Approve on the tablet** when the pairing request appears. Keep the app open. Wait for the Windows badge to change to **trusted**.
5. Then choose files and press **Send**, and tap **Approve** again on the tablet for the transfer.

Never paste the code into the port field; it is numeric-only and must stay `47831`.

A yellow **code needed** badge just means this device has not been paired yet; it is not an active pairing state.

## Transfer is stuck at "Pairing"

During **Pairing**, the Windows sender is waiting for approval on the tablet. Open Shadow Transfer on the tablet and tap **Approve**. Pairing times out after 180 seconds and reports "declined by receiver" if nothing happens. If the tablet is asleep or the app is closed, no approval dialog can appear.

During **Send**, the sender may be waiting for the tablet to approve the incoming transfer; the transfer card says so. Tap **Approve** there as well.

If the code was wrong, the transfer fails with "invalid pairing code". Get a fresh code from the tablet's Receive screen and retry. If it says the peer identity is not trusted, the sender must be re-paired (see the table below).

## The other device does not appear in the list

1. Is **Receive** enabled on the other device? Beacons exist only while receive mode is on.
2. Press **Scan** again. The app also does one 3-second scan at launch, but if the tablet enabled receive later, scan again.
3. Check Windows Firewall: allow Shadow Transfer on **private** networks. A dismissed prompt silently blocks both inbound TCP and discovery.
4. Use **manual IP**: tablet IP + port `47831`. This bypasses discovery entirely.
5. Guest Wi-Fi / AP isolation blocks device-to-device traffic even on the same SSID. Disable isolation or use a phone hotspot.
6. Disable VPNs on both devices. Many VPNs route LAN traffic away or block inbound sockets.
7. Some routers separate 2.4 GHz and 5 GHz into isolated networks.

## Connection times out after selecting a device

- Firewall or AP isolation (see above).
- The advertised port is stale: refresh discovery.
- Windows receive port was changed in Settings; enter `IP:port` with the new port manually.
- Android always listens on 47831.
- Another instance of Shadow Transfer is already bound to the port. Check with `netstat -ano | findstr 47831` and close the other instance.

## Pairing fails

| Message / symptom | Cause | Fix |
|---|---|---|
| Invalid pairing code | Code expired (300 s), was rotated after a previous pairing, or typed incorrectly | Read the current code on the receiver; use **New code** and retype |
| Too many pairing attempts | More than 5 failures in 60 s | Wait a minute, create a new code, retry |
| Peer identity is not trusted | Sender has no pin for the receiver although the receiver still remembers the sender (e.g. sender reinstalled) | On the receiver, revoke the sender in Windows Settings or clear Android app data, then pair again |
| Pairing is disabled | Receiver refused the request | Enable receive mode / restart it |
| Declined | Receiver user pressed Decline or the approval timed out (180 s) | Ask the receiver user to Approve, retry |

Pairing is not available when receive mode is off. There is no bypass and no plaintext fallback by design.

## Sender is trusted but files still fail

- The receiver may have revoked the device in Settings. Pair again.
- Identity files were deleted or changed: a new identity means a new fingerprint and requires pairing.
- Check free space at the destination.
- Check that the source file was not modified after selection (`source file changed during transfer`).
- Read `%LOCALAPPDATA%\ShadowTransfer\shadow.log.*` on Windows.

## Verification failed

The receiver re-hashes the complete file and rejected it. Possible causes: storage corruption, a full disk, or an interrupted write. The transfer stops rather than reporting success. Retry from the sender; the receiver will not offer the corrupt partial as complete and will restart that file. Check disk health/space if it repeats.

## Android: received files are not in my selected folder

- The file is only copied to SAF after verification. If the provider failed, the app shows an error and keeps the verified file in app staging (`getExternalFilesDir(null)/incoming/...`); the UI message includes the path.
- Re-select the destination folder to renew the persisted grant, then retry the transfer.
- The provider may have changed permissions or the folder was deleted. Choose a normal local folder such as Downloads.
- If no folder was chosen, files intentionally stay in staging and the UI shows the path.

## Android: receive mode stops in the background

- Allow notifications and do not force-stop the app. Receive mode uses a `dataSync` foreground service.
- Samsung battery optimization can still kill background apps aggressively. Exclude Shadow Transfer from battery optimization if you need long unattended receives.
- This behavior was not validated on the physical tablet in this build; report exact device/Android version if you test it.

## Android: cannot choose a folder / permission lost

The Storage Access Framework grant is per folder and can be revoked by the system or the provider. Use **Choose folder** again. The app never requests broad storage permission.

## Windows: SmartScreen warning when running

The EXE and installer are not code-signed. SmartScreen may warn. Verify the file origin/hash locally; do not disable SmartScreen globally.

## Windows: app window is blank or closes immediately

- Read `%LOCALAPPDATA%\ShadowTransfer\shadow.log.*`.
- Startup requires read/write access to that folder. Corporate policy or antivirus may block it.
- A GPU/driver issue with OpenGL can prevent the egui window from appearing. Update the graphics driver; software rendering fallbacks are outside this build's scope.
- Delete only `settings.json` to reset settings; deleting `identity.key.der`/`identity.cert.der` creates a new identity and requires re-pairing. Deleting `trusted.json` removes all trusted devices.

## Port already in use

Windows: change the receive port in Settings, save, and re-enable receive mode. Android: stop the other app using 47831 (the receiver port is fixed).

## Transfers are slow

Observed throughput is bounded by the Wi-Fi link. Check:

- Both devices on **5 GHz** (the tablet may have silently joined 2.4 GHz). Windows: `netsh wlan show interfaces` shows band, channel and PHY rate.
- Signal strength. `-62 dBm` is usable but not excellent; move closer or remove obstructions. Ping spikes (hundreds of ms) indicate airtime contention or Wi-Fi power saving.
- Use a 5 GHz channel with 80 MHz width, and avoid busy channels. The 5 GHz-only SSID is preferable.
- If possible, connect the PC by **Ethernet**; the tablet remains the wireless hop.
- Turn off tablet battery saver for Shadow Transfer and keep the app in the foreground. The updated Android APK takes a high-performance Wi-Fi lock **only during transfers**.
- Choose internal storage, not a slow SD card, as the SAF receive folder.
- 1.7 GB in seconds needs >1.4 Gbps. Realistic Wi-Fi 5 tablet throughput is tens of MB/s. The app cannot exceed the link.

The updated Windows build uses 8 MiB TCP buffers and the updated Android build avoids re-reading fresh files before acknowledgement; install both updates and measure again.

## Build problems

| Problem | Fix |
|---|---|
| `rustc`/`cargo` not found | Add `%USERPROFILE%\.cargo\bin` to PATH |
| MSVC `link.exe` missing | Install Visual Studio 2022 C++ build tools |
| Android SDK not found | Create `apps/android/local.properties` with `sdk.dir=C:/Android/Sdk` (forward slashes) |
| NDK missing | `sdkmanager --install "ndk;26.1.10909125"` and set `ANDROID_NDK_HOME` |
| `cargo ndk` missing | `cargo install cargo-ndk --locked` |
| JNI library missing from APK | Re-run the `cargo ndk ... -o ../app/src/main/jniLibs` command before Gradle |
| Release APK unsigned | Add `apps/android/keystore.properties` and the keystore, or build debug only |
| `rc.exe` error building Windows EXE | Install the Windows SDK resource compiler; the build currently only warns and continues without embedded metadata |
| Gradle cannot download dependencies | First build needs internet access to Google Maven and Maven Central |

## Diagnosing with ADB

```
adb devices
adb logcat | findstr -i shadow
adb shell dumpsys activity services com.shadowtransfer.app
adb shell run-as com.shadowtransfer.app ls -l files
adb shell run-as com.shadowtransfer.app ls -l files/../files
```

Rust does not write logcat directly; transfer errors are delivered as UI events and shown in the app / history. Add temporary Kotlin logging around `ShadowEngine.nativePollEvent()` if you need an event trace.

## Reset

- Windows: revoke devices in Settings; to fully reset, close the app and delete `%LOCALAPPDATA%\ShadowTransfer\` (new identity, new trust, empty history). This is irreversible.
- Android: Settings → Apps → Shadow Transfer → Storage → Clear data. This deletes staging, history, identity and trust.

Never send a copy of `identity.key.der` or `release.keystore` to anyone.
