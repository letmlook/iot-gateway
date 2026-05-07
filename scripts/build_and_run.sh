#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────
# IoT Gateway — One-Click Build & Run
# ──────────────────────────────────────────────
# Builds the frontend (Vue/Vite), then the backend (Rust),
# and serves everything from a single backend process.
#
# Usage:
#   ./scripts/build_and_run.sh              # production build + run
#   ./scripts/build_and_run.sh --dev        # dev mode: frontend Vite + backend
#   ./scripts/build_and_run.sh --release    # production build only (no run)
#   ./scripts/build_and_run.sh --frontend   # frontend build only
#   ./scripts/build_and_run.sh --backend    # backend build only
#
# Environment variables (all optional):
#   GATEWAY_PORT       Backend listen port (default: 4000)
#   GATEWAY_DATA_DIR   Data directory (default: data)
#   GATEWAY_STATIC_DIR Static assets dir (default: web/dist)
#   GATEWAY_CONFIG     Gateway config path (default: config/gateway.json)
#   RUST_LOG           Tracing filter (default: info,tower_http=debug)
#   GATEWAY_DISABLE_AUTH  Set to 1/true to skip auth (default: from config)
# ──────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
GATEWAY_PORT="${GATEWAY_PORT:-4000}"
GATEWAY_DATA_DIR="${GATEWAY_DATA_DIR:-data}"
GATEWAY_STATIC_DIR="${GATEWAY_STATIC_DIR:-web/dist}"
GATEWAY_CONFIG="${GATEWAY_CONFIG:-config/gateway.json}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${GREEN}[BUILD]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC}  $*"; }
err()  { echo -e "${RED}[ERROR]${NC} $*" >&2; }
info() { echo -e "${CYAN}[INFO]${NC}  $*"; }

# ─── Parse args ─────────────────────────────
MODE="full"
RUN=true
RELEASE_FLAG="--release"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dev)
      MODE="dev"
      RELEASE_FLAG=""
      ;;
    --release|--build-only)
      RUN=false
      ;;
    --frontend)
      MODE="frontend"
      RUN=false
      ;;
    --backend)
      MODE="backend"
      RUN=false
      ;;
    --debug)
      RELEASE_FLAG=""
      ;;
    -h|--help)
      echo "Usage: $0 [--dev|--release|--frontend|--backend|--debug]"
      echo ""
      echo "  (no flag)   Production build + run backend"
      echo "  --dev       Dev mode: frontend Vite dev server + cargo run (no release build)"
      echo "  --release   Production build, don't run"
      echo "  --frontend  Build frontend only"
      echo "  --backend   Build backend only"
      echo "  --debug     Build backend without --release"
      exit 0
      ;;
    *)
      err "Unknown option: $1"
      exit 1
      ;;
  esac
  shift
done

# ─── Check prerequisites ─────────────────────
check_cmd() {
  command -v "$1" >/dev/null 2>&1 || { err "$1 is required but not found"; exit 1; }
}

cd "$PROJECT_DIR"

# ─── Detect Node.js version ──────────────────
detect_node() {
  local node_ver
  node_ver=$(node -v 2>/dev/null | sed 's/v//' || echo "0")
  local major
  major=$(echo "$node_ver" | cut -d. -f1)

  if [[ "$major" -lt 20 ]]; then
    # Try nvm
    if [ -s "$HOME/.nvm/nvm.sh" ]; then
      . "$HOME/.nvm/nvm.sh"
      if nvm ls 2>/dev/null | grep -q "v20"; then
        local v20
        v20=$(nvm ls 2>/dev/null | grep "v20" | head -1 | awk '{print $1}' | sed 's/[->*[:space:]]//g')
        if [ -n "$v20" ]; then
          warn "Node $node_ver is too old for Vite 7. Switching to $v20 via nvm..."
          nvm use "$v20" >/dev/null 2>&1
          return 0
        fi
      fi
      if nvm ls 2>/dev/null | grep -q "v22"; then
        local v22
        v22=$(nvm ls 2>/dev/null | grep "v22" | head -1 | awk '{print $1}' | sed 's/[->*[:space:]]//g')
        if [ -n "$v22" ]; then
          warn "Node $node_ver is too old for Vite 7. Switching to $v22 via nvm..."
          nvm use "$v22" >/dev/null 2>&1
          return 0
        fi
      fi
    fi
    err "Node.js 20.19+ or 22.12+ required for Vite 7. Found: $node_ver"
    err "Install via nvm: nvm install 20 && nvm use 20"
    exit 1
  fi
}

# ─── Build Frontend ──────────────────────────
build_frontend() {
  log "Building frontend (Vue/Vite)..."
  detect_node
  cd "$PROJECT_DIR/web"

  if [[ ! -d "node_modules" ]]; then
    log "Installing frontend dependencies..."
    npm install --silent
  fi

  npm run build

  local dist="$PROJECT_DIR/web/dist"
  if [[ -f "$dist/index.html" ]]; then
    log "Frontend build complete → $dist"
  else
    err "Frontend build failed — web/dist/index.html not found"
    exit 1
  fi

  cd "$PROJECT_DIR"
}

# ─── Build Backend ───────────────────────────
build_backend() {
  log "Building backend (Rust)..."
  check_cmd cargo

  if [[ -n "$RELEASE_FLAG" ]]; then
    cargo build --release
    local bin="$PROJECT_DIR/target/release/gateway"
  else
    cargo build
    local bin="$PROJECT_DIR/target/debug/gateway"
  fi

  if [[ -f "$bin" ]]; then
    log "Backend build complete → $bin"
  else
    err "Backend build failed — binary not found at $bin"
    exit 1
  fi
}

# ─── Run Backend ─────────────────────────────
run_backend() {
  local bin
  if [[ -n "$RELEASE_FLAG" ]]; then
    bin="$PROJECT_DIR/target/release/gateway"
  else
    bin="$PROJECT_DIR/target/debug/gateway"
  fi

  if [[ ! -f "$bin" ]]; then
    err "Backend binary not found: $bin"
    err "Build first: $0 --backend"
    exit 1
  fi

  # Ensure data directory exists
  mkdir -p "$PROJECT_DIR/$GATEWAY_DATA_DIR"
  mkdir -p "$PROJECT_DIR/logs"

  info "──────────────────────────────────────────"
  info "  IoT Gateway starting..."
  info "  URL:      http://localhost:${GATEWAY_PORT}"
  info "  API:      http://localhost:${GATEWAY_PORT}/api"
  info "  Static:   $GATEWAY_STATIC_DIR"
  info "──────────────────────────────────────────"

  export GATEWAY_PORT
  export GATEWAY_DATA_DIR
  export GATEWAY_STATIC_DIR
  export GATEWAY_CONFIG

  exec "$bin"
}

# ─── Dev Mode ────────────────────────────────
run_dev() {
  detect_node
  info "Starting dev mode..."
  info "  Frontend: http://localhost:5173 (Vite dev server)"
  info "  Backend:  http://localhost:${GATEWAY_PORT} (API)"
  info "  Vite proxies /api → backend"

  # Build and start backend in background
  warn "Building backend (debug mode)..."
  cargo build
  local bin="$PROJECT_DIR/target/debug/gateway"

  export GATEWAY_PORT
  export GATEWAY_DATA_DIR
  export GATEWAY_STATIC_DIR
  export GATEWAY_CONFIG

  log "Starting backend on port ${GATEWAY_PORT}..."
  "$bin" &
  BACKEND_PID=$!
  trap "kill $BACKEND_PID 2>/dev/null; exit" INT TERM EXIT

  # Give backend a moment to start
  sleep 1

  log "Starting Vite dev server..."
  cd "$PROJECT_DIR/web"
  npx vite --host

  # Vite exits → kill backend
  kill $BACKEND_PID 2>/dev/null || true
}

# ─── Main ────────────────────────────────────
case "$MODE" in
  full)
    build_frontend
    build_backend
    if $RUN; then
      run_backend
    else
      log "Build complete (--release, not running)."
      info "Run manually: GATEWAY_PORT=$GATEWAY_PORT ./target/release/gateway"
    fi
    ;;
  dev)
    run_dev
    ;;
  frontend)
    build_frontend
    log "Frontend-only build complete."
    ;;
  backend)
    build_backend
    log "Backend-only build complete."
    ;;
esac
