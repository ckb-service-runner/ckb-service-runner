#!/usr/bin/env bash
# Runs cargo with the user-space Rust toolchain of user aiworker.
set -euo pipefail
if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
export CARGO_TERM_COLOR=never
exec cargo "$@"
