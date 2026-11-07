$ErrorActionPreference = "Stop"
$root = Resolve-Path "$PSScriptRoot\.."
Set-Location $root
New-Item -ItemType Directory -Force -Path "dist" | Out-Null

Write-Host "Running loopback transfer benchmark..."
cargo run --release -p shadow-transfer --example bench_transfer -- "1,10,50,100,500,1024" 1024 |
    Tee-Object -FilePath "dist\performance-loopback.txt"

Write-Host "Running Criterion microbenchmarks..."
cargo bench -p shadow-transfer --bench core |
    Tee-Object -FilePath "dist\performance-criterion.txt"

Write-Host "Reports written to dist\performance-*.txt"
