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

echo "all services built"
