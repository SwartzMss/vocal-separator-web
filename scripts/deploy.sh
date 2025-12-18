#!/usr/bin/env bash
set -euo pipefail

# vocal-separator-web 部署脚本：构建 Rust 后端 + React 前端、同步静态资源、配置 systemd 与 nginx。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${ENV_FILE:-$ROOT/backend/.env}"
FRONTEND_BUILD="$ROOT/frontend/dist"
STATIC_DEST="${STATIC_DEST:-/var/www/vocal-separator-web}"
SERVICE_NAME="vocal-separator-web"
BACKEND_UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"
ORIG_USER="${SUDO_USER:-$(id -un)}"
ORIG_HOME="$(getent passwd "$ORIG_USER" | cut -d: -f6)"

ensure_root() {
  if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
    echo "This script must be run as root (use sudo)." >&2
    exit 1
  fi
}

ensure_root

load_env_file() {
  if [[ -f "$ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    set -a && source "$ENV_FILE" && set +a
  fi
}

load_env_file

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Command '$1' not found. Please install it before running deploy.sh." >&2
    exit 1
  fi
}

require_cmd python3

SERVICE_USER="${SERVICE_USER:-$ORIG_USER}"
SERVICE_GROUP="${SERVICE_GROUP:-$SERVICE_USER}"
SERVICES=("$SERVICE_NAME")
NGINX_SERVICE="${NGINX_SERVICE:-nginx}"
CLIENT_MAX_BODY_SIZE="${CLIENT_MAX_BODY_SIZE:-200M}"

usage() {
  echo "Usage: $0 [install|start|stop|restart|status|build|clean-static|uninstall]" >&2
  exit 1
}

ACTION="${1:-start}"
shift || true

build() {
  if [[ -n "$ORIG_HOME" ]]; then
    export HOME="$ORIG_HOME"
    if [[ -f "$ORIG_HOME/.cargo/env" ]]; then
      # shellcheck disable=SC1090
      source "$ORIG_HOME/.cargo/env"
    fi
    if [[ -s "$ORIG_HOME/.nvm/nvm.sh" ]]; then
      export NVM_DIR="${NVM_DIR:-$ORIG_HOME/.nvm}"
      # shellcheck disable=SC1090
      source "$ORIG_HOME/.nvm/nvm.sh"
    fi
  fi
  bash "$ROOT/scripts/build.sh"
}

sync_static_assets() {
  local static_root="$STATIC_DEST"
  if [[ ! -d "$FRONTEND_BUILD" ]]; then
    echo "frontend build not found at $FRONTEND_BUILD; run build first" >&2
    exit 1
  fi
  mkdir -p "$static_root"
  rsync -a --delete "$FRONTEND_BUILD"/ "$static_root"/
}

clean_static() {
  if [[ -d "$STATIC_DEST" ]]; then
    rm -rf "$STATIC_DEST"
    echo "Removed static assets at $STATIC_DEST"
  else
    echo "Static directory $STATIC_DEST does not exist"
  fi
}

resolve_backend_path() {
  local input="$1"
  if [[ -z "$input" ]]; then
    return
  fi
  if [[ "$input" == /* ]]; then
    printf "%s\n" "$input"
  else
    python3 - "$ROOT/backend" "$input" <<'PY'
import os
import sys
from pathlib import Path
base = Path(sys.argv[1]).resolve()
path = sys.argv[2]
if os.path.isabs(path):
    print(os.path.abspath(path))
else:
    print(os.path.abspath(base / path))
PY
  fi
}

prepare_runtime_dirs() {
  local dir resolved
  for dir in "${JOBS_DIR:-}" "${LOG_DIR:-}"; do
    [[ -z "$dir" ]] && continue
    resolved="$(resolve_backend_path "$dir")"
    mkdir -p "$resolved"
    chown "$SERVICE_USER:$SERVICE_GROUP" "$resolved"
  done
}

read_nginx_vars() {
  DOMAIN="${DOMAIN:-${DEPLOY_DOMAIN:-${VSW_DEPLOY_DOMAIN:-}}}"
  EXTERNAL_PORT="${EXTERNAL_PORT:-${DEPLOY_EXTERNAL_PORT:-${VSW_DEPLOY_EXTERNAL_PORT:-443}}}"
  CERT_PATH="${CERT_PATH:-${DEPLOY_CERT_PATH:-${VSW_SSL_CERT_PATH:-}}}"
  KEY_PATH="${KEY_PATH:-${DEPLOY_KEY_PATH:-${VSW_SSL_KEY_PATH:-}}}"
  BACKEND_BIND="${BACKEND_BIND:-${DEPLOY_BACKEND_BIND:-${VSW_BACKEND_BIND:-}}}"

  if [[ -z "$BACKEND_BIND" ]]; then
    host="${SERVER_HOST:-127.0.0.1}"
    port="${SERVER_PORT:-8000}"
    BACKEND_BIND="${host}:${port}"
  fi

  if [[ -z "${DOMAIN:-}" || -z "${CERT_PATH:-}" || -z "${KEY_PATH:-}" ]]; then
    cat >&2 <<EOF
nginx requires DOMAIN, CERT_PATH, KEY_PATH.
Provide them via environment variables or $ENV_FILE, e.g.:
  DOMAIN=vocal.example.com
  CERT_PATH=/etc/letsencrypt/live/vocal/fullchain.pem
  KEY_PATH=/etc/letsencrypt/live/vocal/privkey.pem
EOF
    exit 1
  fi
}

configure_nginx() {
  read_nginx_vars
  sync_static_assets

  local nginx_conf="/etc/nginx/sites-available/${SERVICE_NAME}.conf"
  cat >"$nginx_conf" <<EOF
server {
    listen 80;
    server_name $DOMAIN;
    return 301 https://\$host:$EXTERNAL_PORT\$request_uri;
}

server {
    listen $EXTERNAL_PORT ssl;
    server_name $DOMAIN;

    ssl_certificate $CERT_PATH;
    ssl_certificate_key $KEY_PATH;
    client_max_body_size $CLIENT_MAX_BODY_SIZE;

    root $STATIC_DEST;
    index index.html;

    location /api/ {
        proxy_pass http://$BACKEND_BIND;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_http_version 1.1;
    }

    location / {
        try_files \$uri /index.html;
    }
}
EOF

  ln -sf "$nginx_conf" /etc/nginx/sites-enabled/${SERVICE_NAME}.conf
}

write_unit_files() {
  tee "$BACKEND_UNIT_PATH" >/dev/null <<EOF
[Unit]
Description=vocal-separator-web backend (Rust)
After=network-online.target
Wants=network-online.target

[Service]
EnvironmentFile=$ENV_FILE
WorkingDirectory=$ROOT/backend
ExecStart=$ROOT/backend/target/release/backend
Restart=on-failure
RestartSec=3
User=$SERVICE_USER
Group=$SERVICE_GROUP

[Install]
WantedBy=multi-user.target
EOF
}

start_services() {
  systemctl daemon-reload
  for svc in "${SERVICES[@]}"; do
    systemctl start "${svc}.service"
  done
}

stop_services() {
  for ((idx=${#SERVICES[@]}-1; idx>=0; idx--)); do
    systemctl stop "${SERVICES[idx]}.service" >/dev/null 2>&1 || true
  done
}

status_services() {
  for svc in "${SERVICES[@]}"; do
    systemctl status "${svc}.service" --no-pager
  done
}

reload_nginx() {
  nginx -t
  systemctl reload "${NGINX_SERVICE}.service"
}

remove_systemd_unit() {
  systemctl stop "${SERVICE_NAME}.service" >/dev/null 2>&1 || true
  systemctl disable "${SERVICE_NAME}.service" >/dev/null 2>&1 || true
  rm -f "$BACKEND_UNIT_PATH"
  systemctl daemon-reload
}

remove_nginx_config() {
  rm -f "/etc/nginx/sites-enabled/${SERVICE_NAME}.conf"
  rm -f "/etc/nginx/sites-available/${SERVICE_NAME}.conf"
  systemctl reload "${NGINX_SERVICE}.service" >/dev/null 2>&1 || true
}

uninstall() {
  echo "Stopping services..."
  stop_services
  echo "Removing systemd unit..."
  remove_systemd_unit
  echo "Removing nginx config..."
  remove_nginx_config
  if [[ -d "$STATIC_DEST" ]]; then
    echo "Removing static assets at $STATIC_DEST"
    rm -rf "$STATIC_DEST"
  fi
  echo "Uninstall completed."
}

case "$ACTION" in
  install)
    stop_services
    build
    write_unit_files
    configure_nginx
    prepare_runtime_dirs
    start_services
    reload_nginx
    ;;
  build)
    build
    ;;
  start)
    build
    write_unit_files
    configure_nginx
    prepare_runtime_dirs
    start_services
    reload_nginx
    ;;
  stop)
    stop_services
    ;;
  restart)
    stop_services
    build
    write_unit_files
    configure_nginx
    prepare_runtime_dirs
    start_services
    reload_nginx
    ;;
  status)
    status_services
    ;;
  clean-static)
    clean_static
    ;;
  uninstall)
    uninstall
    ;;
  *)
    usage
    ;;
esac
