#!/usr/bin/env bash
# csr-windows, phase 1: type-check the runner for x86_64-pc-windows-gnu.
# Prints SKIP (exit 0) only when the toolchain cannot be prepared on this machine.
set -uo pipefail
CARGO="bash acceptance/a1/cargo.sh"
RUSTUP="${HOME}/.cargo/bin/rustup"
TARGET=x86_64-pc-windows-gnu
if [ -x "$RUSTUP" ]; then
  "$RUSTUP" target list --installed 2>/dev/null | grep -qx "$TARGET" || "$RUSTUP" target add "$TARGET" >/dev/null 2>&1
fi
if ! { [ -x "$RUSTUP" ] && "$RUSTUP" target list --installed 2>/dev/null | grep -qx "$TARGET"; }; then
  echo "SKIP: rust target $TARGET not available"; exit 0
fi
out=$($CARGO check -q -p csr --target "$TARGET" 2>&1); code=$?
if [ $code -eq 0 ]; then echo "OK: csr type-checks for $TARGET"; exit 0; fi
if echo "$out" | grep -Eqi "x86_64-w64-mingw32-gcc|failed to find tool|is \`cc\` not installed|error occurred: Command .*gcc"; then
  echo "SKIP: a dependency needs a MinGW C compiler (x86_64-w64-mingw32-gcc) to build for $TARGET"
  echo "$out" | tail -n 5
  exit 0
fi
echo "$out"
exit 1
