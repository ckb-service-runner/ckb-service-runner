#!/usr/bin/env bash
# Builds all CSR contracts (release, riscv64imac-unknown-none-elf) into contracts/build/release/<name>.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
if command -v rustup >/dev/null 2>&1; then
  rustup target add riscv64imac-unknown-none-elf >/dev/null
fi
TARGET=riscv64imac-unknown-none-elf
LOCKED=""
[ -f "$ROOT/contracts/Cargo.lock" ] && LOCKED="--locked"
OUT="$ROOT/contracts/build/release"
mkdir -p "$OUT"
build() {
  local args=""
  for c in "$@"; do args="$args -p $c"; done
  cargo build --manifest-path "$ROOT/contracts/Cargo.toml" --release --target $TARGET -j 2 $LOCKED $args \
    --config "target.$TARGET.rustflags=['-C','link-arg=-s']"
  for c in "$@"; do
    cp "$ROOT/contracts/target/$TARGET/release/$c" "$OUT/$c"
    echo "  $c: $(stat -c %s "$OUT/$c") bytes"
  done
}
echo "== CSR contracts =="
# csr-registry embeds the code hash of the built csr-dual-lock (csr-registry/build.rs), so the dual lock comes first.
build csr-dual-lock
build csr-float-lock csr-registry csr-job-type csr-job-lock
