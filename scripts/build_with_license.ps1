# 编译网关并自动生成 license（与后端一致的机器码逻辑，输出到 data/license.dat）
# 用法：在项目根目录执行 .\scripts\build_with_license.ps1 [--release]

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$release = $args -contains "--release"
if ($release) {
    cargo build --release -p gateway-server
} else {
    cargo build -p gateway-server
}
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# 生成与当前机器一致的 license 到 data/
python scripts/gen_license_auto.py
if ($LASTEXITCODE -ne 0) {
    Write-Host "gen_license_auto failed (pip install cryptography? scripts/license_private.pem?)" -ForegroundColor Yellow
    exit $LASTEXITCODE
}
Write-Host "License written to data\license.dat"
