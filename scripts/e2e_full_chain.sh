#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PYTHON_BIN="${PYTHON_BIN:-python3}"
REQUIREMENTS="$ROOT_DIR/e2e/full_chain/requirements.txt"
RUNNER="$ROOT_DIR/e2e/full_chain/test_full_chain.py"

if ! command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  echo "[fail] python3 is required; set PYTHON_BIN=/path/to/python if needed" >&2
  exit 1
fi

if [[ ! -x "${GATEWAY_BIN:-}" ]]; then
  if [[ -x "$ROOT_DIR/target/debug/gateway" ]]; then
    export GATEWAY_BIN="$ROOT_DIR/target/debug/gateway"
  elif [[ -x "$ROOT_DIR/target/release/gateway" ]]; then
    export GATEWAY_BIN="$ROOT_DIR/target/release/gateway"
  else
    echo "[skip] gateway binary not found; run: cargo build -p gateway-server" >&2
    exit 77
  fi
fi

if ! "$PYTHON_BIN" - <<'PY' >/dev/null 2>&1
import aiohttp
import websockets
import paho.mqtt.client
PY
then
  echo "[fail] missing Python dependencies; run:" >&2
  echo "       $PYTHON_BIN -m pip install -r $REQUIREMENTS" >&2
  exit 1
fi

cd "$ROOT_DIR"
exec "$PYTHON_BIN" "$RUNNER" "$@"
