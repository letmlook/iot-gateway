#!/usr/bin/env pwsh
<#
.SYNOPSIS
IoT Gateway — One-Click Build & Run (Windows)

.DESCRIPTION
Builds the frontend (Vue/Vite), then the backend (Rust),
and serves everything from a single backend process.

.PARAMETER Dev
Run in dev mode: Vite dev server + backend

.PARAMETER Release
Build only, don't run

.PARAMETER Frontend
Build frontend only

.PARAMETER Backend
Build backend only

.PARAMETER Debug
Build backend without --release flag

.EXAMPLE
.\scripts\build_and_run.ps1
.\scripts\build_and_run.ps1 -Dev
.\scripts\build_and_run.ps1 -Release
#>

param(
  [switch]$Dev,
  [switch]$Release,
  [switch]$Frontend,
  [switch]$Backend,
  [switch]$Debug
)

$ErrorActionPreference = "Stop"
$ProjectDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

$env:GATEWAY_PORT = if ($env:GATEWAY_PORT) { $env:GATEWAY_PORT } else { "4000" }
$env:GATEWAY_DATA_DIR = if ($env:GATEWAY_DATA_DIR) { $env:GATEWAY_DATA_DIR } else { "data" }
$env:GATEWAY_STATIC_DIR = if ($env:GATEWAY_STATIC_DIR) { $env:GATEWAY_STATIC_DIR } else { "web/dist" }
$env:GATEWAY_CONFIG = if ($env:GATEWAY_CONFIG) { $env:GATEWAY_CONFIG } else { "config/gateway.json" }

$Port = $env:GATEWAY_PORT

function Write-Build { Write-Host "[BUILD] $args" -ForegroundColor Green }
function Write-Warn  { Write-Host "[WARN]  $args" -ForegroundColor Yellow }
function Write-Error { Write-Host "[ERROR] $args" -ForegroundColor Red }
function Write-Info  { Write-Host "[INFO]  $args" -ForegroundColor Cyan }

# ─── Check Node.js version ───────────────────
function Check-NodeVersion {
  $nodeVer = (node -v 2>$null) -replace 'v', ''
  if (-not $nodeVer) {
    Write-Error "Node.js is required but not found"
    exit 1
  }
  $major = [int]($nodeVer.Split('.')[0])
  if ($major -lt 20) {
    # Try fnm
    if (Get-Command fnm -ErrorAction SilentlyContinue) {
      Write-Warn "Node $nodeVer is too old. Trying fnm..."
      fnm use 20 2>$null
      if ($LASTEXITCODE -ne 0) {
        fnm install 20
        fnm use 20
      }
      return
    }
    # Try nvm-windows
    if (Get-Command nvm -ErrorAction SilentlyContinue) {
      Write-Warn "Node $nodeVer is too old. Trying nvm-windows..."
      nvm use 20.19.0 2>$null
      if ($LASTEXITCODE -ne 0) {
        nvm install 20.19.0
        nvm use 20.19.0
      }
      return
    }
    Write-Error "Node.js 20.19+ required. Found: $nodeVer"
    exit 1
  }
}

# ─── Build Frontend ──────────────────────────
function Build-Frontend {
  Write-Build "Building frontend (Vue/Vite)..."
  Check-NodeVersion
  Set-Location "$ProjectDir/web"

  if (-not (Test-Path "node_modules")) {
    Write-Build "Installing frontend dependencies..."
    npm install
  }

  npm run build

  $dist = "$ProjectDir/web/dist"
  if (Test-Path "$dist/index.html") {
    Write-Build "Frontend build complete → $dist"
  } else {
    Write-Error "Frontend build failed — web/dist/index.html not found"
    exit 1
  }

  Set-Location $ProjectDir
}

# ─── Build Backend ───────────────────────────
function Build-Backend {
  Write-Build "Building backend (Rust)..."
  $releaseFlag = if (-not $Debug) { "--release" } else { "" }

  if ($releaseFlag) {
    cargo build --release
    $bin = "$ProjectDir\target\release\gateway.exe"
  } else {
    cargo build
    $bin = "$ProjectDir\target\debug\gateway.exe"
  }

  if (Test-Path $bin) {
    Write-Build "Backend build complete → $bin"
  } else {
    Write-Error "Backend build failed — binary not found at $bin"
    exit 1
  }
}

# ─── Run Backend ─────────────────────────────
function Start-Backend {
  if (-not $Debug) {
    $bin = "$ProjectDir\target\release\gateway.exe"
  } else {
    $bin = "$ProjectDir\target\debug\gateway.exe"
  }

  if (-not (Test-Path $bin)) {
    Write-Error "Backend binary not found: $bin"
    exit 1
  }

  New-Item -ItemType Directory -Force -Path "$ProjectDir\$($env:GATEWAY_DATA_DIR)" | Out-Null
  New-Item -ItemType Directory -Force -Path "$ProjectDir\logs" | Out-Null

  Write-Info "──────────────────────────────────────────"
  Write-Info "  IoT Gateway starting..."
  Write-Info "  URL:      http://localhost:$Port"
  Write-Info "  API:      http://localhost:$Port/api"
  Write-Info "  Static:   $($env:GATEWAY_STATIC_DIR)"
  Write-Info "──────────────────────────────────────────"

  & $bin
}

# ─── Dev Mode ────────────────────────────────
function Start-DevMode {
  Check-NodeVersion
  Write-Info "Starting dev mode..."
  Write-Info "  Frontend: http://localhost:5173 (Vite dev server)"
  Write-Info "  Backend:  http://localhost:$Port (API)"

  $bin = "$ProjectDir\target\debug\gateway.exe"
  if (-not (Test-Path $bin)) {
    Write-Warn "Building backend (debug mode)..."
    cargo build
  }

  $backendJob = Start-Job -ScriptBlock {
    param($bin, $env)
    foreach ($kv in $env.GetEnumerator()) {
      [Environment]::SetEnvironmentVariable($kv.Key, $kv.Value, "Process")
    }
    & $bin
  } -ArgumentList $bin, (Get-ChildItem env:)

  try {
    Start-Sleep -Seconds 1
    Set-Location "$ProjectDir/web"
    npx vite --host
  } finally {
    Stop-Job -Job $backendJob -ErrorAction SilentlyContinue
    Remove-Job -Job $backendJob -ErrorAction SilentlyContinue
    Write-Info "Dev mode stopped."
  }
}

# ─── Main ────────────────────────────────────
Set-Location $ProjectDir

if ($Dev) {
  Start-DevMode
} elseif ($Frontend) {
  Build-Frontend
} elseif ($Backend) {
  Build-Backend
} elseif ($Release) {
  Build-Frontend
  Build-Backend
  Write-Build "Build complete (--release, not running)."
  Write-Info "Run manually: `$env:GATEWAY_PORT=$Port .\target\release\gateway.exe"
} else {
  Build-Frontend
  Build-Backend
  Start-Backend
}
