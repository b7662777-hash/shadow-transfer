$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
Set-Location $root

Write-Host "Building Shadow Transfer for Windows (release)..."
cargo build --release -p shadow-windows
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $dist "docs") | Out-Null

Copy-Item "target\release\ShadowTransfer.exe" (Join-Path $dist "ShadowTransfer.exe") -Force

$portable = Join-Path $dist "ShadowTransfer-0.1.0-portable"
if (Test-Path $portable) { Remove-Item $portable -Recurse -Force }
New-Item -ItemType Directory -Force -Path $portable | Out-Null
Copy-Item "target\release\ShadowTransfer.exe" (Join-Path $portable "ShadowTransfer.exe") -Force
Copy-Item "README.md" (Join-Path $portable "README.md") -Force
Copy-Item "LICENSE" (Join-Path $portable "LICENSE") -Force
Copy-Item "docs\WINDOWS.md" (Join-Path $portable "WINDOWS.md") -Force
Compress-Archive -Path (Join-Path $portable "*") -DestinationPath (Join-Path $dist "ShadowTransfer-0.1.0-portable.zip") -Force

Write-Host "Windows artifacts written to $dist"
Write-Host "Run scripts\build-installer.ps1 to produce ShadowTransfer-Setup.exe"
