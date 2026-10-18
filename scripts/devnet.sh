#!/usr/bin/env bash
# Local CKB devnet harness: start | mine N | status | stop | reset
set -u

CKB_VERSION="0.210.0"
CKB_SHA256="68a94c191109bae172f8dcb71828dbdf0383e2c314e0658743db217472b27bdf"
CKB_ARCHIVE="ckb_v${CKB_VERSION}_x86_64-unknown-linux-gnu.tar.gz"
CKB_URL="https://github.com/nervosnetwork/ckb/releases/download/v${CKB_VERSION}/${CKB_ARCHIVE}"

RPC_HOST=127.0.0.1
RPC_PORT=18114
P2P_PORT=18115
# Miner key of the devnet (public test key, see devtools/devnet-keys.json).
BA_ARGS="0x470dcdc5e44064909650113a274b3b36aecb6dc7"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="$ROOT/.tools"
DEVNET="$ROOT/.devnet"
CKB="$TOOLS/ckb-v${CKB_VERSION}/ckb"
PIDFILE="$DEVNET/ckb.pid"
RPC_URL="http://$RPC_HOST:$RPC_PORT"

die() { echo "devnet: $*" >&2; exit "${2:-1}"; }

rpc() { # method params-json
  curl -sS --max-time 20 -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-[]}}" "$RPC_URL"
}

is_running() {
  [ -f "$PIDFILE" ] || return 1
  local pid; pid="$(cat "$PIDFILE" 2>/dev/null)"
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && grep -q ckb "/proc/$pid/comm" 2>/dev/null
}

port_used() { (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; }

ensure_ckb() {
  [ -x "$CKB" ] && return 0
  command -v curl >/dev/null || die "curl is required"
  mkdir -p "$TOOLS"
  local tmp; tmp="$(mktemp -d "$TOOLS/dl.XXXXXX")" || die "cannot create temp dir in $TOOLS"
  echo "devnet: downloading ckb v$CKB_VERSION ..."
  curl -fsSL -o "$tmp/$CKB_ARCHIVE" "$CKB_URL" || { rm -rf "$tmp"; die "download failed"; }
  local got; got="$(sha256sum "$tmp/$CKB_ARCHIVE" | cut -d' ' -f1)"
  [ "$got" = "$CKB_SHA256" ] || { rm -rf "$tmp"; die "SHA-256 mismatch for $CKB_ARCHIVE (got $got)"; }
  mkdir "$tmp/x" && tar xzf "$tmp/$CKB_ARCHIVE" -C "$tmp/x" || { rm -rf "$tmp"; die "extract failed"; }
  local dir; dir="$(find "$tmp/x" -maxdepth 1 -mindepth 1 -type d | head -n1)"
  [ -x "$dir/ckb" ] || { rm -rf "$tmp"; die "ckb binary missing in archive"; }
  rm -rf "$TOOLS/ckb-v${CKB_VERSION}"
  mv "$dir" "$TOOLS/ckb-v${CKB_VERSION}" && rm -rf "$tmp"
}

init_devnet() {
  [ -f "$DEVNET/ckb.toml" ] && return 0
  rm -rf "$DEVNET"; mkdir -p "$DEVNET"
  "$CKB" init --chain dev -C "$DEVNET" --ba-arg "$BA_ARGS" --ba-message 0x --force >/dev/null 2>&1 \
    || { rm -rf "$DEVNET"; die "ckb init failed"; }
  local f="$DEVNET/ckb.toml"
  sed -i \
    -e "s|^listen_addresses = .*|listen_addresses = [\"/ip4/127.0.0.1/tcp/$P2P_PORT\"]|" \
    -e "s|^bootnodes = .*|bootnodes = []|" \
    -e "s|^listen_address = \"127.0.0.1:8114\"|listen_address = \"$RPC_HOST:$RPC_PORT\"|" \
    -e 's|^modules = \["Net".*|modules = ["Net", "Pool", "Miner", "Chain", "Stats", "Experiment", "Debug", "Indexer", "IntegrationTest"]|' \
    "$f"
  grep -q '^\[block_assembler\]' "$f" || die "block assembler not configured by ckb init"
  grep -q "$P2P_PORT" "$f" && grep -q "$RPC_PORT" "$f" && grep -q IntegrationTest "$f" \
    || { rm -rf "$DEVNET"; die "failed to patch ckb.toml"; }
}

cmd_start() {
  if is_running; then echo "devnet: already running"; return 0; fi
  port_used "$RPC_PORT" && die "port $RPC_PORT is in use by another process; refusing to start"
  port_used "$P2P_PORT" && die "port $P2P_PORT is in use by another process; refusing to start"
  ensure_ckb
  init_devnet
  cd "$DEVNET" || die "cannot enter $DEVNET"
  setsid nohup "$CKB" run -C "$DEVNET" >"$DEVNET/ckb.log" 2>&1 </dev/null &
  echo $! >"$PIDFILE"
  disown
  local i
  for i in $(seq 1 120); do
    is_running || { tail -n 20 "$DEVNET/ckb.log" >&2; die "ckb exited during startup"; }
    rpc get_tip_block_number >/dev/null 2>&1 && break
    sleep 0.5
  done
  rpc get_tip_block_number >/dev/null 2>&1 || die "RPC did not come up on $RPC_URL"
  cmd_mine 1 || die "initial block failed"
  echo "devnet: running (rpc $RPC_URL)"
}

cmd_mine() {
  local n="${1:-}"
  [[ "$n" =~ ^[0-9]+$ ]] || die "usage: devnet.sh mine N"
  local i out
  for ((i = 0; i < n; i++)); do
    out="$(rpc generate_block)" || die "generate_block failed (node not running?)"
    case "$out" in *'"error"'*) die "generate_block: $out";; esac
  done
}

cmd_status() { if is_running; then echo "running"; return 0; else echo "not running"; return 3; fi; }

cmd_stop() {
  if is_running; then
    local pid; pid="$(cat "$PIDFILE")"
    kill "$pid" 2>/dev/null
    local i
    for i in $(seq 1 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done
    kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null
  fi
  rm -f "$PIDFILE"
  return 0
}

case "${1:-}" in
  start) cmd_start ;;
  mine) shift; cmd_mine "${1:-}" ;;
  status) cmd_status ;;
  stop) cmd_stop ;;
  reset) cmd_stop; rm -rf "$DEVNET" ;;
  *) echo "usage: devnet.sh start|mine N|status|stop|reset" >&2; exit 2 ;;
esac
