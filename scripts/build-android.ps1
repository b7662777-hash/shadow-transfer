$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
$android = Join-Path $root "apps\android"
$ndk = "C:\Android\Sdk\ndk\26.1.10909125"

if (-not (Test-Path $ndk)) {
    throw "NDK not found at $ndk. Install it with: sdkmanager --install 'ndk;26.1.10909125'"
}
$env:ANDROID_NDK_HOME = $ndk

Write-Host "Building Rust JNI libraries..."
Set-Location (Join-Path $android "rust")
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o ../app/src/main/jniLibs --platform 26 build --release -p shadow-android
if ($LASTEXITCODE -ne 0) { throw "cargo ndk failed" }

Write-Host "Building APKs..."
Set-Location $android
& .\gradlew.bat :app:assembleDebug :app:assembleRelease --console=plain
if ($LASTEXITCODE -ne 0) { throw "Gradle build failed" }

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item "app\build\outputs\apk\debug\app-debug.apk" (Join-Path $dist "ShadowTransfer-0.1.0-debug.apk") -Force
Copy-Item "app\build\outputs\apk\release\app-release.apk" (Join-Path $dist "ShadowTransfer-0.1.0-release.apk") -Force

Write-Host "APKs written to $dist"
Write-Host "Install with: adb install -r dist\ShadowTransfer-0.1.0-release.apk"
