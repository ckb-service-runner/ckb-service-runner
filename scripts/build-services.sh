#!/usr/bin/env bash
# Builds every CSR service. Idempotent; stops at the first failure.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

echo "== Rust services =="
if command -v rustup >/dev/null 2>&1; then
  rustup target add riscv64imac-unknown-none-elf >/dev/null
fi
LOCKED=""
[ -f "$ROOT/services/rust/Cargo.lock" ] && LOCKED="--locked"
(cd "$ROOT/services/rust" && cargo build --release $LOCKED)

echo "== C services =="
command -v clang >/dev/null 2>&1 || { echo "clang is required to build the C services" >&2; exit 1; }
# clang drives the link with ld.lld: the system one, else the Rust toolchain's rust-lld.
LLD=$(command -v ld.lld || true)
if [ -z "$LLD" ]; then
  RUST_LLD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/rust-lld"
  [ -x "$RUST_LLD" ] || { echo "neither ld.lld nor rust-lld found" >&2; exit 1; }
  mkdir -p "$ROOT/.tools/bin"
  ln -sf "$RUST_LLD" "$ROOT/.tools/bin/ld.lld"
  LLD="$ROOT/.tools/bin/ld.lld"
fi
for dir in "$ROOT"/services/c/*/; do
  [ -f "$dir/Makefile" ] || continue
  make -C "$dir" --no-print-directory LDEXTRA="-fuse-ld=$LLD"
done

echo "== JS services =="
# ckb-js-vm is committed (services/vendor); the JS services are plain source files.
(cd "$ROOT/services/vendor" && grep -v '^#' SHA256SUMS | sha256sum -c --strict -)
# The args of a ckb-js-vm manifest name the JS cell by its data hash; it must match the file.
check_js_args() {
  python3 -I - "$1" "$2" <<'EOF'
import hashlib, re, sys
manifest, js = sys.argv[1], sys.argv[2]
m = re.search(r'^args\s*=\s*"0x([0-9a-fA-F]*)"', open(manifest).read(), re.M)
h = hashlib.blake2b(open(js, "rb").read(), digest_size=32, person=b"ckb-default-hash").hexdigest()
expected = "0000" + h + "04"
if not m or m.group(1).lower() != expected:
    sys.exit(f"{manifest}: args must be 0x{expected} (flags, data hash of {js}, data2)")
EOF
}
check_js_args "$ROOT/services/manifests/hello-js.toml" "$ROOT/services/js/hello/hello.js"

echo "all services built"
