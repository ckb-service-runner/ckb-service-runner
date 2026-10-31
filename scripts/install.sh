#!/usr/bin/env bash
# CSR installer for Linux x86_64 with systemd (tested on Ubuntu/Debian). Testnet only.
#
#   curl -fLO https://raw.githubusercontent.com/ckb-service-runner/ckb-service-runner/main/scripts/install.sh
#   less install.sh          # read it first
#   sudo bash install.sh
#
# What it does: downloads the latest release and checks its SHA-256, installs /usr/local/bin/csr, creates the
# `csr` system user and /var/lib/csr, writes /var/lib/csr/csr.toml (only if it does not exist yet), sets up the
# node-identity proof, installs and starts the `csr` service and the signed-update helper, asks for a dashboard
# password. It never changes the CKB node, its ckb.toml or its data; it only grants the `csr` user read access
# (an ACL) to the node's P2P key file.
#
# Running it again upgrades the binary and service files and keeps csr.toml, keys and data.
#
# Options:
#   --key PATH       the node's P2P key (<ckb data dir>/network/secret_key); default: search for it
#   --no-identity    skip the node-identity proof
#   --inbound        ping_mode "inbound" (needs TCP 8131 reachable from the internet); default "outbound"
#   --no-password    do not ask for a dashboard password
#   --rpc URL        node RPC (loopback only); default http://127.0.0.1:8114
#   -y, --yes        do not ask questions (takes the first key found; no password)
#   -h, --help       this text
set -euo pipefail

REPO="ckb-service-runner/ckb-service-runner"
BASE="https://github.com/${REPO}/releases/latest/download"
DATA=/var/lib/csr
CONF="$DATA/csr.toml"
BIN=/usr/local/bin/csr
UNIT_DIR=/etc/systemd/system

RPC="http://127.0.0.1:8114"
RPC_GIVEN=0
KEY=""
IDENTITY=1
PING_MODE="outbound"
PASSWORD=1
YES=0
TMP=""

# ---------------------------------------------------------------- output helpers
if [ -t 1 ]; then B=$'\e[1m'; G=$'\e[32m'; Y=$'\e[33m'; R=$'\e[31m'; N=$'\e[0m'; else B=""; G=""; Y=""; R=""; N=""; fi
step() { printf '\n%s==> %s%s\n' "$B" "$*" "$N"; }
ok()   { printf '  %s✓%s %s\n' "$G" "$N" "$*"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$*"; }
die()  { printf '\n%sError:%s %s\n' "$R" "$N" "$*" >&2; exit 1; }
ask()  { # ask "question" default(y|n) -> 0 for yes
  local a
  if [ "$YES" = 1 ] || [ ! -t 0 ]; then [ "$2" = y ]; return; fi
  read -r -p "  $1 [$( [ "$2" = y ] && echo Y/n || echo y/N )] " a
  a="${a:-$2}"; [[ "$a" =~ ^[Yy] ]]
}
usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0; }

csr_as() { sudo -u csr "$BIN" --data-root "$DATA" "$@"; }

# ---------------------------------------------------------------- steps
parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --key) KEY="${2:?--key needs a path}"; shift ;;
      --no-identity) IDENTITY=0 ;;
      --inbound) PING_MODE="inbound" ;;
      --no-password) PASSWORD=0 ;;
      --rpc) RPC="${2:?--rpc needs a URL}"; RPC_GIVEN=1; shift ;;
      -y|--yes) YES=1 ;;
      -h|--help) usage ;;
      *) die "unknown option: $1 (see --help)" ;;
    esac
    shift
  done
  case "$RPC" in
    http://127.0.0.1:*|http://localhost:*|http://\[::1\]:*) ;;
    *) die "--rpc must be a loopback http URL (127.0.0.1, localhost or [::1])" ;;
  esac
}

check_system() {
  step "Checking this machine"
  [ "$(id -u)" = 0 ] || die "run as root: sudo bash $0"
  [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || die "CSR needs Linux on x86_64 (this is $(uname -s) $(uname -m))"
  command -v systemctl >/dev/null && [ -d /run/systemd/system ] || die "systemd is required"
  local missing=() c
  for c in curl sha256sum tar sudo awk; do command -v "$c" >/dev/null || missing+=("$c"); done
  if [ "$IDENTITY" = 1 ] && ! command -v setfacl >/dev/null; then missing+=(acl); fi
  if [ ${#missing[@]} -gt 0 ]; then
    command -v apt-get >/dev/null || die "please install: ${missing[*]}"
    warn "installing ${missing[*]}"
    apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${missing[@]/sha256sum/coreutils}" >/dev/null
  fi
  ok "Linux x86_64 with systemd"
}

rpc_call() { # rpc_call method -> JSON result on stdout; fails on no answer or a JSON-RPC error
  local out  # a disabled module still answers HTTP 200, with {"error":…}
  out="$(curl -fsS -m 5 -H 'content-type: application/json' \
    -d "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":[]}" "$RPC")" || return 1
  case "$out" in *'"error":'*|'') return 1 ;; esac
  printf '%s' "$out"
}

check_node() {
  step "Checking the CKB node at $RPC"
  rpc_call get_tip_block_number >/dev/null 2>&1 || die "no CKB node answers at $RPC.
       CSR must run on the SAME machine as your CKB full node (it only talks to a local node and reads
       the node's key file). If your node runs elsewhere, run this script on that machine and open the
       dashboard from here with: ssh -L 8130:127.0.0.1:8130 <user>@<node machine>
       If the node is on this machine: start it, or pass --rpc if its RPC port is not 8114."
  ok "node answers"
  if rpc_call local_node_info >/dev/null 2>&1; then ok "Net module enabled"
  else warn "the node's Net RPC module is off: node identity and NAT help need it (add \"Net\" to modules in ckb.toml)"; fi
  if ! rpc_call get_indexer_tip >/dev/null 2>&1; then
    die "the node's Indexer RPC module is off. In ckb.toml, under [rpc], add \"Indexer\" to modules
       (and keep \"Net\"), restart the node and run this script again."
  fi
  ok "Indexer module enabled"
  case "$(rpc_call get_blockchain_info 2>/dev/null | grep -o '"chain":"[^"]*"' | cut -d'"' -f4)" in
    ckb_testnet) ok "chain: testnet" ;;
    ckb) die "this node runs on mainnet. The installer sets up testnet only; for mainnet follow docs/install.md." ;;
    *) warn "could not read the chain name; continuing (csr check below verifies it)" ;;
  esac
}

install_binary() {
  step "Downloading the latest CSR release"
  local tmp tarball dir
  TMP="$(mktemp -d)"; tmp="$TMP"; trap 'rm -rf "$TMP"' EXIT
  curl -fsSL -o "$tmp/SHA256SUMS" "$BASE/SHA256SUMS" || die "cannot download from GitHub ($BASE)"
  tarball="$(awk '/linux-x86_64\.tar\.gz$/ {print $2}' "$tmp/SHA256SUMS")"
  [ -n "$tarball" ] || die "the release has no Linux tarball"
  curl -fsSL -o "$tmp/$tarball" "$BASE/$tarball" || die "download of $tarball failed"
  (cd "$tmp" && grep "  $tarball\$" SHA256SUMS | sha256sum -c --quiet -) || die "checksum of $tarball does not match"
  ok "$tarball: checksum OK"
  tar -C "$tmp" -xzf "$tmp/$tarball"
  dir="$tmp/${tarball%.tar.gz}"
  local old=""; [ -x "$BIN" ] && old="$("$BIN" version 2>/dev/null || true)"
  install -m 0755 -o root -g root "$dir/csr" "$BIN"
  ok "installed $("$BIN" version)${old:+ (was: $old)}"
  # service files and the signed-update helper
  install -m 0644 "$dir/csr.service" "$UNIT_DIR/csr.service"
  install -d -m 0755 /usr/local/lib/csr
  install -m 0755 -o root -g root "$dir/install-update.sh" /usr/local/lib/csr/install-update.sh
  install -m 0644 "$dir/csr-update.path" "$dir/csr-update.service" "$UNIT_DIR/"
  install -m 0644 "$dir/csr.example.toml" /usr/local/lib/csr/csr.example.toml
  ok "service files and update helper installed"
}

setup_user_and_config() {
  step "User and configuration"
  if id csr >/dev/null 2>&1; then ok "user csr exists"
  else useradd --system --home-dir "$DATA" --shell /usr/sbin/nologin csr; ok "user csr created"; fi
  install -d -o csr -g csr -m 0750 "$DATA"
  if [ -f "$CONF" ]; then
    ok "keeping your existing $CONF"
    if [ "$RPC_GIVEN" = 1 ]; then
      sed -i -E "s|^rpc_url[[:space:]]*=[[:space:]]*\"[^\"]*\"|rpc_url = \"$RPC\"|" "$CONF"
      ok "rpc_url set to $RPC"
    fi
    return
  fi
  install -m 0640 -o csr -g csr /usr/local/lib/csr/csr.example.toml "$CONF"
  sed -i \
    -e "s|^rpc_url = \"[^\"]*\"|rpc_url = \"$RPC\"|" \
    -e "s|^ping_mode = \"[^\"]*\"|ping_mode = \"$PING_MODE\"|" \
    "$CONF"
  ok "wrote $CONF (testnet, ping_mode = \"$PING_MODE\")"
}

find_keys() { # prints candidate P2P key files, one per line
  local roots=() r
  for r in /var/lib /home /root /opt /srv /data /mnt /media /usr/local; do [ -d "$r" ] && roots+=("$r"); done
  find "${roots[@]}" -xdev -maxdepth 8 -type f -path '*/network/secret_key' 2>/dev/null | sort -u
}

set_key_in_config() { # set_key_in_config PATH
  if grep -qE '^[#[:space:]]*p2p_key_file[[:space:]]*=' "$CONF"; then
    sed -i -E "s|^[#[:space:]]*p2p_key_file[[:space:]]*=.*|p2p_key_file = \"$1\"|" "$CONF"
  else
    sed -i -E "s|^(rpc_url[[:space:]]*=.*)$|\1\np2p_key_file = \"$1\"|" "$CONF"
  fi
}

grant_key_access() { # grant_key_access PATH
  setfacl -m u:csr:r "$1"
  local d; d="$(dirname "$1")"
  while [ "$d" != "/" ]; do setfacl -m u:csr:x "$d"; d="$(dirname "$d")"; done
}

hidden_by_unit() { # is PATH under a directory that the unit's ProtectHome= hides?
  case "$1" in /home/*|/root/*|/run/user/*) return 0 ;; *) return 1 ;; esac
}

write_dropin() { # make the key visible to the service when it lives under /home or /root
  local dropin="$UNIT_DIR/csr.service.d/node-key.conf"
  if hidden_by_unit "$1"; then
    install -d -m 0755 "$UNIT_DIR/csr.service.d"
    printf '# Written by scripts/install.sh: the node key lives under a home directory.\n[Service]\nProtectHome=tmpfs\nBindReadOnlyPaths=%s\n' "$1" > "$dropin"
    ok "service may read the key under $(dirname "$1") (systemd drop-in)"
  else
    rm -f "$dropin"
  fi
}

identity_state() { # `csr check` exits 2 while the node is not synced: still read its output
  { csr_as check 2>/dev/null || true; } | sed -n 's/^node identity: //p' | head -1
}

setup_identity() {
  [ "$IDENTITY" = 1 ] || { warn "node identity skipped (--no-identity): the monitor will not rank this node"; return; }
  step "Node identity (lets the CSR monitor rank your node)"
  local configured; configured="$(sed -nE 's/^p2p_key_file[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$CONF" | head -1)"
  local candidates=()
  if [ -n "$KEY" ]; then candidates=("$KEY")
  elif [ -n "$configured" ]; then candidates=("$configured")
  else mapfile -t candidates < <(find_keys)
  fi
  if [ ${#candidates[@]} -eq 0 ]; then
    warn "no CKB P2P key found. Run again with --key <ckb data dir>/network/secret_key"
    return
  fi
  local k state
  for k in "${candidates[@]}"; do
    [ -f "$k" ] || { warn "$k: not a file"; continue; }
    if [ "$(stat -c %a "$k" | cut -c3)" != 0 ]; then
      warn "$k is readable by every user; CSR refuses such a key. Fix with: sudo chmod o-rwx '$k'"; continue
    fi
    if [ ${#candidates[@]} -gt 1 ] && ! ask "use the key $k ?" y; then continue; fi
    grant_key_access "$k"
    set_key_in_config "$k"
    state="$(identity_state)"
    case "$state" in
      proven)
        write_dropin "$k"; ok "node identity: proven ($k)"; return ;;
      mismatch)
        setfacl -x u:csr "$k" 2>/dev/null || true
        warn "$k belongs to another node (mismatch), skipped" ;;
      *) # the node did not answer: keep the key, CSR proves the identity once the node is up
        write_dropin "$k"
        warn "node identity ${state:-unknown} for $k: kept; CSR proves it once the node answers"
        return ;;
    esac
  done
  warn "node identity not proven; the node is not ranked. Fix later: see docs/install.md, \"Node identity\""
}

check_runner() {
  step "Checking CSR against the node"
  csr_as config validate >/dev/null || die "the configuration is invalid: sudo -u csr $BIN --data-root $DATA config validate"
  ok "configuration valid"
  local out r2; out="$(csr_as check 2>&1 || true)"
  r2="$(printf '%s\n' "$out" | sed -n 's/^R2 state: //p')"
  if [ "$r2" = Synced ]; then ok "node is synced"
  else
    warn "node state: ${r2:-unknown}. CSR starts anyway and begins work once the node is synced:"
    printf '%s\n' "$out" | sed -n 's/^  - /      /p'
  fi
}

start_services() {
  step "Starting CSR"
  systemctl daemon-reload
  systemctl enable --quiet csr csr-update.path
  systemctl restart csr
  systemctl restart csr-update.path
  sleep 3
  systemctl is-active --quiet csr || die "csr did not start. Log: sudo journalctl -u csr -n 50 --no-pager"
  ok "csr is running and starts with the machine"
  ok "automatic update helper enabled"
}

setup_password() {
  [ "$PASSWORD" = 1 ] || return 0
  if grep -qE '^password_hash[[:space:]]*=[[:space:]]*"[^"]+' "$CONF"; then ok "dashboard password already set"; return; fi
  step "Dashboard password"
  if [ "$YES" = 1 ] || [ ! -t 0 ]; then
    warn "no password set. Without one the dashboard is read-only. Later: sudo -u csr $BIN --data-root $DATA set-password"
    return
  fi
  echo "  Without a password the dashboard is read-only (no updates, wallet, registration or settings)."
  if csr_as set-password; then systemctl restart csr; ok "password set"
  else warn "password not set; run later: sudo -u csr $BIN --data-root $DATA set-password"; fi
}

summary() {
  local id; id="$(identity_state)"
  printf '\n%sCSR is installed.%s\n\n' "$B" "$N"
  printf '  Dashboard     http://127.0.0.1:8130  (on this machine)\n'
  printf '  From another  ssh -L 8130:127.0.0.1:8130 %s@%s   then open http://127.0.0.1:8130\n' "${SUDO_USER:-user}" "$(hostname)"
  printf '  Identity      %s\n' "${id:-unknown}"
  printf '  Logs          sudo journalctl -u csr -f\n'
  printf '  Config        %s  (then: sudo systemctl restart csr)\n' "$CONF"
  printf '\n  Next (optional): register to run service jobs. See the dashboard page "Wallet & registration"\n'
  printf '  and https://github.com/%s#runner-wallet-stake-and-registration\n\n' "$REPO"
}

main() {
  parse_args "$@"
  check_system
  check_node
  install_binary
  setup_user_and_config
  setup_identity
  check_runner
  start_services
  setup_password
  summary
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then main "$@"; fi
