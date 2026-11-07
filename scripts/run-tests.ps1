$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
Set-Location $root

Write-Host "cargo fmt --check"
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw "formatting check failed" }

Write-Host "cargo clippy -D warnings"
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw "clippy failed" }

Write-Host "cargo test --workspace"
cargo test --workspace
if ($LASTEXITCODE -ne 0) { throw "tests failed" }

Write-Host "All quality gates passed."
