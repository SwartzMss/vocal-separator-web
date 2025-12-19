#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BACKEND_DIR="${BACKEND_DIR:-$ROOT/backend}"
FRONTEND_DIR="${FRONTEND_DIR:-$ROOT/frontend}"
PY_REQUIREMENTS_FILE="${PY_REQUIREMENTS_FILE:-}"

if [[ -z "$PY_REQUIREMENTS_FILE" ]]; then
  if [[ -f "$ROOT/python/requirements.txt" ]]; then
    PY_REQUIREMENTS_FILE="$ROOT/python/requirements.txt"
  else
    PY_REQUIREMENTS_FILE="$ROOT/requirements.txt"
  fi
fi

require_cmd() {
  for cmd in "$@"; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      echo "error: missing required command: $cmd" >&2
      exit 1
    fi
  done
}

echo "==> Checking prerequisites"
require_cmd cargo npm python3 pip

echo "==> Backend (cargo fmt/clippy/build)"
pushd "$BACKEND_DIR" >/dev/null
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
popd >/dev/null

echo "==> Frontend (npm install + npm run build)"
pushd "$FRONTEND_DIR" >/dev/null
npm install --prefer-offline --no-audit --no-fund
npm run build -- --emptyOutDir=false
popd >/dev/null

if [[ -f "$PY_REQUIREMENTS_FILE" ]]; then
  echo "==> Python dependencies (global pip)"
  pip install --upgrade pip
  pip install -r "$PY_REQUIREMENTS_FILE"
fi

echo "All checks/builds completed successfully."
