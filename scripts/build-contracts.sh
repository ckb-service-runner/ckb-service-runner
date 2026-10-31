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
# CARGO_TARGET_DIR: e.g. the contract tests' build.rs builds into its own directory (no lock on contracts/target)
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/contracts/target}"
mkdir -p "$OUT"
build() {
  local args=""
  for c in "$@"; do args="$args -p $c"; done
  cargo build --manifest-path "$ROOT/contracts/Cargo.toml" --release --target $TARGET -j 2 $LOCKED $args \
    --config "target.$TARGET.rustflags=['-C','link-arg=-s']"
  for c in "$@"; do
    cp "$TARGET_DIR/$TARGET/release/$c" "$OUT/$c"
    echo "  $c: $(stat -c %s "$OUT/$c") bytes"
  done
}
echo "== CSR contracts =="
# csr-registry embeds the code hashes of the built csr-dual-lock and csr-config-type, csr-job-lock the one of
# csr-config-type (build.rs), so those two come first.
build csr-dual-lock
# csr-config-type uses ckb_std::type_id (ckb-types, bytes): its atomics are RISC-V A instructions, so it is
# referenced with hash type data2 (VM version 2, which has the A extension).
build csr-config-type
# Test-only: a byte-different build of the job lock (another embedded constant) for the contract tests
# (job_two_lock_versions_share_refund_rejected). Never deployed.
mkdir -p "$ROOT/contracts/build/test"
CSR_JOB_LOCK_VARIANT=test-variant build csr-job-lock
mv "$OUT/csr-job-lock" "$ROOT/contracts/build/test/csr-job-lock-variant"
build csr-registry csr-job-type csr-job-lock
