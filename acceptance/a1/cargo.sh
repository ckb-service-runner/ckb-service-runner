#!/usr/bin/env bash
# Runs cargo with the user-space Rust toolchain of user aiworker.
set -euo pipefail
if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
export CARGO_TERM_COLOR=never
# The service crates live in their own workspace (services/rust): run there when one is selected with -p.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
for a in "$@"; do
  case "$a" in
    csr-svc|crowdcell-finalize|crowdcell-finalize-logic) cd "$root/services/rust"; break ;;
  esac
done
exec cargo "$@"
