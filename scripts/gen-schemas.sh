#!/usr/bin/env bash
# Regenerates contracts/csr-schemas/src/generated.rs from contracts/schemas/csr.mol.
# Needs `moleculec` (cargo install moleculec --locked --root .tools).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$PWD/.tools/bin:$PATH"
moleculec --language rust --schema-file contracts/schemas/csr.mol > contracts/csr-schemas/src/generated.rs
echo "wrote contracts/csr-schemas/src/generated.rs"
