# Build

## Environment used for this build

| Tool | Version / location |
|---|---|
| OS | Windows 11 10.0.26200, x64 |
| CPU / RAM / disk | Intel Core i5-1235U / 24 GB / NVMe |
| Rust | 1.99.1 stable (MSVC), installed via rustup |
| Visual Studio | 2022 Community with MSVC C++ tools |
| Java | Temurin OpenJDK 17.0.20 |
| Android SDK | `C:\Android\Sdk`, platform 34, build-tools 34.0.0 |
| Android NDK | 26.1.10909125 (r26b) |
| Gradle | 8.7 (wrapper committed after generation) |
| AGP / Kotlin | 8.5.2 / 1.9.24, Compose compiler 1.5.14 |
| cargo-ndk | installed with `cargo install cargo-ndk --locked` |
| Inno Setup | 6.7.3 (user install) |

## Rust workspace

```
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Release builds use thin LTO, one codegen unit, stripped symbols and unwind panics (JNI and tests need unwind).

## Windows

```
cargo build --release -p shadow-windows
```

Output: `target/release/ShadowTransfer.exe` (~8.4 MiB).

The binary embeds a generated icon, product name/description/version via `winresource`; the window uses the same icon. If `rc.exe` is missing from the MSVC installation, the resource step logs a warning and still produces a working executable without embedded metadata.

Portable package creation:

```
powershell -File scripts/build-windows.ps1
```

Installer:

```
"C:\Users\<you>\AppData\Local\Programs\Inno Setup 6\ISCC.exe" apps\windows\installer\ShadowTransfer.iss
```

or `powershell -File scripts/build-installer.ps1`. Output: `dist/ShadowTransfer-Setup.exe`. The installer script includes `README.md` and `docs/WINDOWS.md`.

## Android

### One-time toolchain setup

```
sdkmanager --install "ndk;26.1.10909125"
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk --locked
```

`apps/android/local.properties` must point at the SDK:

```
sdk.dir=C:/Android/Sdk
```

It is already present and ignored by `.gitignore`.

### Native library

From `apps/android/rust`:

```
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
  -o ../app/src/main/jniLibs --platform 26 \
  build --release -p shadow-android
```

The `.so` files are packaged by Gradle. Re-run this after any Rust change affecting Android.

### APKs

```
cd apps/android
./gradlew :app:assembleDebug
./gradlew :app:assembleRelease
```

Outputs:

- `app/build/outputs/apk/debug/app-debug.apk` (application id suffix `.debug`)
- `app/build/outputs/apk/release/app-release.apk`

Release signing is read from `apps/android/keystore.properties` if present:

```
storeFile=release.keystore
storePassword=<local password>
keyAlias=shadowtransfer
keyPassword=<local password>
```

Both the keystore and properties file are local build artifacts (gitignored). If they are removed, the release APK builds unsigned. Generate a new keystore with:

```
keytool -genkeypair -v -keystore release.keystore -alias shadowtransfer \
  -keyalg RSA -keysize 4096 -validity 10000 \
  -storepass <password> -keypass <password> \
  -dname "CN=Shadow Transfer, O=Shadow Transfer, C=US"
```

Use `scripts/build-android.ps1` to rebuild native libs and both APKs in one step.

## Benchmarks

```
cargo run --release -p shadow-transfer --example bench_transfer -- "1,10,50,100,500,1024" 1024
cargo bench -p shadow-transfer --bench core
```

Set the first argument to `5120` for the 5 GiB case. See PERFORMANCE.md.

## Artifacts

After a full build, copy artifacts to `dist/`:

```
dist/
├── ShadowTransfer.exe            (portable, also see portable zip)
├── ShadowTransfer-Setup.exe
├── ShadowTransfer-0.1.0-portable.zip
├── ShadowTransfer-0.1.0-debug.apk
└── ShadowTransfer-0.1.0-release.apk
```

`scripts/build-windows.ps1` and `scripts/build-android.ps1` populate these locations.

## Reproducibility notes

- Cargo resolves the latest compatible versions within the workspace ranges; `Cargo.lock` is committed for repeatable dependency sets.
- Gradle dependency versions are pinned by the Compose BOM and explicit version strings.
- The Android release keystore is local. A rebuild requires it to update the same installed app.
