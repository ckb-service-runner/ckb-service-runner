#!/usr/bin/env bash
# CSR installer for Linux x86_64 with systemd (tested on Ubuntu/Debian).
#
# It finds the CKB full nodes on this machine and installs one CSR instance per network (D31, D32):
#   testnet -> service csr@testnet, /var/lib/csr/csr-testnet.toml, dashboard 8130, ping 8131
#   mainnet -> service csr@mainnet, /var/lib/csr/csr-mainnet.toml, dashboard 8140, ping 8141
# On mainnet CSR is a node dashboard only: the Service Runtime stays off until the Phase 2 release (1.x).
#
#   curl -fLO https://raw.githubusercontent.com/ckb-service-runner/ckb-service-runner/main/scripts/install.sh
#   less install.sh          # read it first
#   sudo bash install.sh
#
# What it does: downloads the latest release and checks its SHA-256, installs /usr/local/bin/csr, creates the
# `csr` system user and /var/lib/csr, writes /var/lib/csr/csr-<network>.toml per node (only if it does not
# exist yet), sets up the node-identity proof, installs and starts csr@<network> and its signed-update helper,
# asks for a dashboard password. An older single install (/var/lib/csr/csr.toml, csr.service) is migrated to
# csr-<network>.toml and csr@<network>, keeping its settings and ports. It never changes the CKB node, its ckb.toml or its data; it only grants the `csr` user read access
# (an ACL) to the node's P2P key file.
#
# Running it again upgrades the binary and service files and keeps the configs, keys and data.
#
# Options:
#   --key PATH       the node's P2P key (<ckb data dir>/network/secret_key); default: search for it
#                    (only used when one node is found; with two nodes each one's key is searched)
#   --no-identity    skip the node-identity proof
#   --inbound        ping_mode "inbound" (needs TCP 8131 reachable from the internet); default "outbound"
#   --no-runtime     write [runtime] enabled = false; default true (runs only services you approve)
#   --no-password    do not ask for a dashboard password
#   --rpc URL        node RPC (loopback only); may be given twice (testnet and mainnet node).
#                    Default: 127.0.0.1:8114 plus the [rpc] listen_address of every ckb.toml found
#   -y, --yes        do not ask questions (takes the first key found; no password)
#   -h, --help       this text
set -euo pipefail

REPO="ckb-service-runner/ckb-service-runner"
BASE="https://github.com/${REPO}/releases/latest/download"
DATA=/var/lib/csr
CONF=""                     # config of the instance being set up: $DATA/csr-$NET.toml
NET=""                      # network of the instance being set up
EXAMPLE=/usr/local/lib/csr/csr.example.toml
BIN=/usr/local/bin/csr
UNIT_DIR=/etc/systemd/system

RPC="http://127.0.0.1:8114"  # node of the instance being set up
RPCS=()
DEFAULT_RPCS=("http://127.0.0.1:8114")
NODES=()                    # "network url" lines from find_nodes
KEY=""
IDENTITY=1
PING_MODE="outbound"
RUNTIME=true
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

csr_as() { sudo -u csr "$BIN" --data-root "$DATA" --config "$CONF" "$@"; }
own_csr() { chown csr:csr "$1"; chmod 0640 "$1"; }

# ---------------------------------------------------------------- steps
parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --key) KEY="${2:?--key needs a path}"; shift ;;
      --no-identity) IDENTITY=0 ;;
      --inbound) PING_MODE="inbound" ;;
      --no-runtime) RUNTIME=false ;;
      --no-password) PASSWORD=0 ;;
      --rpc) RPCS+=("${2:?--rpc needs a URL}"); shift ;;
      -y|--yes) YES=1 ;;
      -h|--help) usage ;;
      *) die "unknown option: $1 (see --help)" ;;
    esac
    shift
  done
  local u
  for u in "${RPCS[@]}"; do
    case "$u" in
      http://127.0.0.1:*|http://localhost:*|http://\[::1\]:*) ;;
      *) die "--rpc must be a loopback http URL (127.0.0.1, localhost or [::1]): $u" ;;
    esac
  done
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

rpc_call_at() { # rpc_call_at URL method -> JSON result on stdout; fails on no answer or a JSON-RPC error
  local out  # a disabled module still answers HTTP 200, with {"error":…}
  out="$(curl -fsS -m 5 -H 'content-type: application/json' \
    -d "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"$2\",\"params\":[]}" "$1")" || return 1
  case "$out" in *'"error":'*|'') return 1 ;; esac
  printf '%s' "$out"
}
rpc_call() { rpc_call_at "$RPC" "$1"; }

chain_network() { # CKB chain name -> CSR network
  case "$1" in ckb) echo mainnet ;; ckb_testnet) echo testnet ;; *) return 1 ;; esac
}

instance_ports() { # NET -> "dashboard ping"
  case "$1" in mainnet) echo "8140 8141" ;; *) echo "8130 8131" ;; esac
}

rpc_url_from_ckb_toml() { # the node's RPC URL from [rpc] listen_address, if it is reachable on loopback
  local addr
  addr="$(awk '
    /^[[:space:]]*\[/ { sec = $0; sub(/^[[:space:]]*/, "", sec); sub(/[[:space:]]*(#.*)?$/, "", sec); next }
    sec == "[rpc]" && /^[[:space:]]*listen_address[[:space:]]*=/ {
      v = $0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/".*$/, "", v); print v; exit
    }' "$1" 2>/dev/null)"
  [ -n "$addr" ] || return 1
  case "${addr%:*}" in
    127.0.0.1|0.0.0.0|localhost) echo "http://127.0.0.1:${addr##*:}" ;;
    *) return 1 ;;
  esac
}

configured_rpc_urls() { # rpc_url of every existing CSR config in $DATA (loopback http only)
  local f u
  for f in "$DATA"/csr.toml "$DATA"/csr-*.toml; do
    [ -f "$f" ] || continue
    u="$(sed -nE 's/^rpc_url[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$f" | head -1)"
    case "$u" in http://127.0.0.1:*|http://localhost:*|http://\[::1\]:*) echo "$u" ;; esac
  done
}

find_nodes() { # prints "testnet URL" and/or "mainnet URL" (first node per network wins)
  local cands=() u k seen=" " chain net tn="" mn=""
  if [ ${#RPCS[@]} -gt 0 ]; then
    cands=("${RPCS[@]}")
  else
    cands=("${DEFAULT_RPCS[@]}")
    # nodes an existing CSR install already uses (csr.toml of an older install, csr-<network>.toml)
    while IFS= read -r u; do if [ -n "$u" ]; then cands+=("$u"); fi; done < <(configured_rpc_urls)
    while IFS= read -r k; do
      [ -n "$k" ] || continue
      if u="$(rpc_url_from_ckb_toml "${k%/data/network/secret_key}/ckb.toml")"; then cands+=("$u"); fi
    done < <(find_keys)
  fi
  for u in "${cands[@]}"; do
    case "$seen" in *" $u "*) continue ;; esac
    seen="$seen$u "
    chain="$(rpc_call_at "$u" get_blockchain_info 2>/dev/null | grep -o '"chain":"[^"]*"' | cut -d'"' -f4)" || true
    net="$(chain_network "$chain")" || continue
    if [ "$net" = testnet ] && [ -z "$tn" ]; then tn="$u"
    elif [ "$net" = mainnet ] && [ -z "$mn" ]; then mn="$u"
    fi
  done
  [ -n "$tn$mn" ] || return 1
  [ -z "$tn" ] || echo "testnet $tn"
  [ -z "$mn" ] || echo "mainnet $mn"
}

discover_nodes() {
  step "Looking for CKB full nodes on this machine"
  local out
  out="$(find_nodes)" || die "no CKB node answers on this machine.
       CSR must run on the SAME machine as your CKB full node (it only talks to a local node and reads
       the node's key file). If your node runs elsewhere, run this script on that machine and open the
       dashboard from here with: ssh -L 8130:127.0.0.1:8130 <user>@<node machine>
       If the node is on this machine: start it, or pass --rpc http://127.0.0.1:<port> (its RPC port)."
  mapfile -t NODES <<< "$out"
  local l; for l in "${NODES[@]}"; do ok "${l%% *} node at ${l#* }"; done
}

check_node() { # checks the node of the current instance ($RPC)
  step "Checking the $NET node at $RPC"
  if rpc_call local_node_info >/dev/null 2>&1; then ok "Net module enabled"
  else warn "the node's Net RPC module is off: node identity and NAT help need it (add \"Net\" to modules in ckb.toml)"; fi
  if ! rpc_call get_indexer_tip >/dev/null 2>&1; then
    die "the $NET node's Indexer RPC module is off. In ckb.toml, under [rpc], add \"Indexer\" to modules
       (and keep \"Net\"), restart the node and run this script again."
  fi
  ok "Indexer module enabled"
}

release_has_templates() { # release_has_templates DIR: the unpacked release ships the per-network units
  [ -f "$1/csr@.service" ] && [ -f "$1/csr-update@.path" ] && [ -f "$1/csr-update@.service" ]
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
  release_has_templates "$dir" || die "the latest release ($tarball) is older than this installer: it has no
       csr@.service template (needs CSR 0.5.0 or later). Nothing was changed. Wait for the next release,
       or use the installer of that release: https://github.com/${REPO}/releases"
  local old=""; [ -x "$BIN" ] && old="$("$BIN" version 2>/dev/null || true)"
  install -m 0755 -o root -g root "$dir/csr" "$BIN"
  ok "installed $("$BIN" version)${old:+ (was: $old)}"
  # service files (one template per network instance) and the signed-update helper
  install -m 0644 "$dir/csr@.service" "$dir/csr-update@.path" "$dir/csr-update@.service" "$UNIT_DIR/"
  install -d -m 0755 /usr/local/lib/csr
  install -m 0755 -o root -g root "$dir/install-update.sh" /usr/local/lib/csr/install-update.sh
  install -m 0644 "$dir/csr.example.toml" "$EXAMPLE"
  ok "service files and update helper installed"
}

setup_user() {
  step "User and data directory"
  if id csr >/dev/null 2>&1; then ok "user csr exists"
  else useradd --system --home-dir "$DATA" --shell /usr/sbin/nologin csr; ok "user csr created"; fi
  install -d -o csr -g csr -m 0750 "$DATA"
}

migrate_legacy() { # an older single install: csr.toml + csr.service -> csr-<network>.toml + csr@<network>
  local old="$DATA/csr.toml" net
  [ -f "$old" ] || return 0
  net="$(sed -nE 's/^network[[:space:]]*=[[:space:]]*"([a-z]+)".*/\1/p' "$old" | head -1)"
  net="${net:-testnet}"
  if [ -f "$DATA/csr-$net.toml" ]; then
    warn "both $old and $DATA/csr-$net.toml exist; left both as they are (csr-$net.toml is used)"
    return 0
  fi
  mv "$old" "$DATA/csr-$net.toml"
  systemctl disable --now csr.service csr-update.path >/dev/null 2>&1 || true
  ok "migrated $old to $DATA/csr-$net.toml (service csr@$net replaces csr.service)"
}

write_instance_config() { # write_instance_config NET URL
  local net="$1" url="$2" f="$DATA/csr-$1.toml" dp pp
  if [ -f "$f" ]; then
    sed -i -E "s|^rpc_url[[:space:]]*=[[:space:]]*\"[^\"]*\"|rpc_url = \"$url\"|" "$f"
    ok "keeping your existing $f (rpc_url = $url)"
    return
  fi
  read -r dp pp < <(instance_ports "$net")
  cp "$EXAMPLE" "$f"
  sed -i \
    -e "s|^network = \"[^\"]*\"|network = \"$net\"|" \
    -e "s|^rpc_url = \"[^\"]*\"|rpc_url = \"$url\"|" \
    -e "/^\[dashboard\]/,/^\[/ s|^bind = \"[^\"]*\"|bind = \"127.0.0.1:$dp\"|" \
    -e "/^\[ranking\]/,/^\[/ s|^ping_bind = \"[^\"]*\"|ping_bind = \"0.0.0.0:$pp\"|" \
    -e "s|^ping_mode = \"[^\"]*\"|ping_mode = \"$PING_MODE\"|" \
    -e "/^\[runtime\]/,/^\[/ s|^enabled = [a-z]*|enabled = $RUNTIME|" \
    "$f"
  own_csr "$f"
  ok "wrote $f ($net, dashboard $dp, ping $pp, ping_mode = \"$PING_MODE\", runtime enabled = $RUNTIME)"
}

set_peer_url() { # set_peer_url NET URL: link this instance's dashboard to the other one
  local f="$DATA/csr-$1.toml"
  if awk '/^[[:space:]]*\[/{sec=$0} sec ~ /^\[dashboard\]/ && /^peer_url[[:space:]]*=/ {f=1} END{exit !f}' "$f"; then
    sed -i "/^\[dashboard\]/,/^\[/ s|^peer_url[[:space:]]*=.*|peer_url = \"$2\"|" "$f"
  else
    sed -i "/^\[dashboard\]/a peer_url = \"$2\"" "$f"
  fi
}

dashboard_port() { # the port of [dashboard] bind in csr-NET.toml
  sed -n '/^\[dashboard\]/,/^\[/ s/^bind = "[^"]*:\([0-9]*\)".*/\1/p' "$DATA/csr-$1.toml" | head -1
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

clear_key_in_config() { # comment out p2p_key_file, so a rejected key never stays configured
  sed -i -E 's|^p2p_key_file[[:space:]]*=.*|# p2p_key_file = ""|' "$CONF"
}

keys_for_node() { # keys_for_node URL: the key of the node whose ckb.toml serves URL first, then all other keys
  local k u first=() rest=()
  while IFS= read -r k; do
    [ -n "$k" ] || continue
    u="$(rpc_url_from_ckb_toml "${k%/data/network/secret_key}/ckb.toml" 2>/dev/null)" || u=""
    if [ -n "$u" ] && [ "$u" = "$1" ]; then first+=("$k"); else rest+=("$k"); fi
  done < <(find_keys)
  if [ ${#first[@]} -gt 0 ]; then printf '%s\n' "${first[@]}"; return; fi
  [ ${#rest[@]} -eq 0 ] || printf '%s\n' "${rest[@]}"
}

grant_key_access() { # grant_key_access PATH
  setfacl -m u:csr:r "$1"
  local d; d="$(dirname "$1")"
  while [ "$d" != "/" ]; do setfacl -m u:csr:x "$d"; d="$(dirname "$d")"; done
}

hidden_by_unit() { # is PATH under a directory that the unit's ProtectHome= hides?
  case "$1" in /home/*|/root/*|/run/user/*) return 0 ;; *) return 1 ;; esac
}

write_dropin() { # make the key visible to the instance when it lives under /home or /root
  local dropin="$UNIT_DIR/csr@$NET.service.d/node-key.conf"
  if hidden_by_unit "$1"; then
    install -d -m 0755 "$UNIT_DIR/csr@$NET.service.d"
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
  step "Node identity of the $NET node (lets the CSR monitor rank it)"
  local configured; configured="$(sed -nE 's/^p2p_key_file[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$CONF" | head -1)"
  local candidates=()
  if [ -n "$KEY" ] && [ ${#NODES[@]} -eq 1 ]; then candidates=("$KEY")
  elif [ -n "$configured" ]; then candidates=("$configured")
  else mapfile -t candidates < <(keys_for_node "$RPC")
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
        clear_key_in_config
        warn "$k belongs to another node (mismatch), skipped" ;;
      unreadable)
        setfacl -x u:csr "$k" 2>/dev/null || true
        clear_key_in_config
        warn "$k cannot be read by the csr user (unreadable), skipped" ;;
      *) # the node did not answer: keep the key, CSR proves the identity once the node is up
        write_dropin "$k"
        warn "node identity ${state:-unknown} for $k: kept; CSR proves it once the node answers"
        return ;;
    esac
  done
  clear_key_in_config
  warn "node identity not proven; the node is not ranked. Fix later: see docs/install.md, \"Node identity\""
}

check_runner() {
  step "Checking CSR against the $NET node"
  csr_as config validate >/dev/null || die "the configuration is invalid: sudo -u csr $BIN --data-root $DATA --config $CONF config validate"
  ok "configuration valid"
  local out r2; out="$(csr_as check 2>&1 || true)"
  r2="$(printf '%s\n' "$out" | sed -n 's/^R2 state: //p')"
  if [ "$r2" = Synced ]; then ok "node is synced"
  else
    warn "node state: ${r2:-unknown}. CSR starts anyway and begins work once the node is synced:"
    printf '%s\n' "$out" | sed -n 's/^  - /      /p'
  fi
}

confirm_mainnet() {
  [ "$NET" = mainnet ] || return 0
  warn "mainnet: CSR runs as a node dashboard only; the Service Runtime stays off until the Phase 2 release (1.x)"
  csr_as confirm-mainnet >/dev/null || die "could not confirm mainnet: sudo -u csr $BIN --data-root $DATA --config $CONF confirm-mainnet"
  ok "mainnet confirmed for csr@mainnet"
}

start_services() { # start_services NET
  step "Starting csr@$1"
  systemctl daemon-reload
  systemctl enable --quiet "csr@$1.service" "csr-update@$1.path"
  systemctl restart "csr@$1.service"
  systemctl restart "csr-update@$1.path"
  sleep 3
  systemctl is-active --quiet "csr@$1.service" || die "csr@$1 did not start. Log: sudo journalctl -u csr@$1 -n 50 --no-pager"
  ok "csr@$1 is running and starts with the machine"
  ok "automatic update helper csr-update@$1.path enabled"
}

setup_password() {
  [ "$PASSWORD" = 1 ] || return 0
  if grep -qE '^password_hash[[:space:]]*=[[:space:]]*"[^"]+' "$CONF"; then ok "$NET dashboard password already set"; return; fi
  step "Dashboard password ($NET)"
  if [ "$YES" = 1 ] || [ ! -t 0 ]; then
    warn "no password set. Without one the dashboard is read-only. Later: sudo -u csr $BIN --data-root $DATA --config $CONF set-password"
    return
  fi
  echo "  Without a password the dashboard is read-only (no updates, wallet, registration or settings)."
  if csr_as set-password; then systemctl restart "csr@$NET.service"; ok "password set"
  else warn "password not set; run later: sudo -u csr $BIN --data-root $DATA --config $CONF set-password"; fi
}

summary() {
  local l id port
  printf '\n%sCSR is installed.%s\n' "$B" "$N"
  for l in "${NODES[@]}"; do
    NET="${l%% *}"; CONF="$DATA/csr-$NET.toml"
    port="$(dashboard_port "$NET")"; id="$(identity_state)"
    printf '\n  %s%s%s (csr@%s)\n' "$B" "$NET" "$N" "$NET"
    printf '  Dashboard     http://127.0.0.1:%s  (on this machine)\n' "$port"
    printf '  From another  ssh -L %s:127.0.0.1:%s %s@%s   then open http://127.0.0.1:%s\n' "$port" "$port" "${SUDO_USER:-user}" "$(hostname)" "$port"
    printf '  Identity      %s\n' "${id:-unknown}"
    if [ "$NET" = mainnet ]; then
      printf '  Runtime       inactive on mainnet until the Phase 2 release (node dashboard and ranking only)\n'
    else
      printf '  Runtime       %s\n' "$(sed -n '/^\[runtime\]/,/^\[/ s/^enabled = //p' "$CONF" | sed 's/ .*//; s/^true$/on (runs only services you approve on the Services page)/; s/^false$/off (switch it on in the dashboard)/; s/^$/as set on the Settings page/')"
    fi
    printf '  Logs          sudo journalctl -u csr@%s -f\n' "$NET"
    printf '  Config        %s  (then: sudo systemctl restart csr@%s)\n' "$CONF" "$NET"
  done
  printf '\n  Next (optional, testnet): register to run service jobs. See the dashboard page "Wallet & registration"\n'
  printf '  and https://github.com/%s#runner-wallet-stake-and-registration\n\n' "$REPO"
}

setup_instance() { # setup_instance NET URL
  NET="$1"; RPC="$2"; CONF="$DATA/csr-$NET.toml"
  check_node
  write_instance_config "$NET" "$RPC"
  setup_identity
  confirm_mainnet
  check_runner
  start_services "$NET"
  setup_password
}

main() {
  parse_args "$@"
  check_system
  discover_nodes
  install_binary
  setup_user
  migrate_legacy
  local l
  for l in "${NODES[@]}"; do setup_instance "${l%% *}" "${l#* }"; done
  if [ ${#NODES[@]} -eq 2 ]; then
    step "Linking the two dashboards"
    set_peer_url testnet "http://127.0.0.1:$(dashboard_port mainnet)"
    set_peer_url mainnet "http://127.0.0.1:$(dashboard_port testnet)"
    systemctl restart csr@testnet.service csr@mainnet.service
    ok "each dashboard links to the other"
  fi
  summary
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then main "$@"; fi
