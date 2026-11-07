$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
Set-Location $root

$candidates = @(
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe"
)
$iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    throw "Inno Setup 6 not found. Install it (winget install JRSoftware.InnoSetup) or compile apps\windows\installer\ShadowTransfer.iss manually."
}

if (-not (Test-Path "target\release\ShadowTransfer.exe")) {
    throw "Build the Windows app first: powershell -File scripts\build-windows.ps1"
}

Write-Host "Compiling installer with $iscc"
& $iscc "apps\windows\installer\ShadowTransfer.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed" }

Write-Host "Installer written to dist\ShadowTransfer-Setup.exe"
