#!/usr/bin/env bash
# Installs a CSR update that the runner staged in <update dir> (run as root by csr-update.service).
#
# The staged files belong to the unprivileged csr user, so they are untrusted: they are read as that
# user into a root-only work directory, and the INSTALLED binary (/usr/local/bin/csr, trusted) checks
# the copies (`csr update verify`: release signature, SHA-256, newer version) before they are
# installed. The previous binary is kept as /usr/local/bin/csr.prev for a rollback.
# No network access. Messages go to stdout/stderr, i.e. the journal (journalctl -u csr-update).
set -euo pipefail

DIR="${1:?usage: install-update.sh <update dir>}"
BIN=/usr/local/bin/csr
PREV=/usr/local/bin/csr.prev
FILES=(csr.new SHA256SUMS SHA256SUMS.sig ready.json)

log() { echo "csr-update: $*" >&2; }

[ -d "$DIR" ] && [ ! -L "$DIR" ] || { log "$DIR is not a directory"; exit 1; }
# Every file operation inside the update directory runs as its owner (normally csr), so that
# symlinks planted there cannot make root read or write other files.
OWNER="$(stat -c %U "$DIR")"
as_owner() { runuser -u "$OWNER" -- "$@"; }

reject() {
  log "update rejected: $*"
  as_owner mkdir -p "$DIR/rejected"
  for f in "${FILES[@]}"; do
    as_owner mv -f "$DIR/$f" "$DIR/rejected/$f" 2>/dev/null || true
  done
  log "staged files moved to $DIR/rejected/"
  exit 1
}

[ -e "$DIR/ready.json" ] || { log "nothing staged in $DIR"; exit 0; }

WORK="$(mktemp -d /run/csr-update.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
for f in "${FILES[@]}"; do
  as_owner cat "$DIR/$f" > "$WORK/$f" || reject "cannot read $f"
done

if ! OUT="$(/usr/local/bin/csr update verify --dir "$WORK" --json 2>&1)"; then
  reject "verification by the installed binary failed: $OUT"
fi
log "verified: $OUT"

cp -p "$BIN" "$PREV"
install -m 0755 -o root -g root "$WORK/csr.new" "$BIN.installing"
mv -f "$BIN.installing" "$BIN"
for f in "${FILES[@]}"; do
  as_owner rm -f "$DIR/$f"
done

if NEW="$(as_owner "$BIN" --version 2>&1)"; then
  log "installed $NEW (previous binary kept as $PREV)"
else
  log "the new binary does not run ($NEW); restoring $PREV"
  install -m 0755 -o root -g root "$PREV" "$BIN.installing"
  mv -f "$BIN.installing" "$BIN"
fi
systemctl restart csr
log "csr restarted"
