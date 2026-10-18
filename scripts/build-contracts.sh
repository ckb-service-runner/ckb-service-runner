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
CONTRACTS="csr-float-lock csr-dual-lock csr-registry csr-job-type csr-job-lock"
ARGS=""
for c in $CONTRACTS; do ARGS="$ARGS -p $c"; done
LOCKED=""
[ -f "$ROOT/contracts/Cargo.lock" ] && LOCKED="--locked"
echo "== CSR contracts =="
cargo build --manifest-path "$ROOT/contracts/Cargo.toml" --release --target $TARGET -j 2 $LOCKED $ARGS \
  --config "target.$TARGET.rustflags=['-C','link-arg=-s']"
OUT="$ROOT/contracts/build/release"
mkdir -p "$OUT"
for c in $CONTRACTS; do
  cp "$ROOT/contracts/target/$TARGET/release/$c" "$OUT/$c"
  echo "  $c: $(stat -c %s "$OUT/$c") bytes"
done
