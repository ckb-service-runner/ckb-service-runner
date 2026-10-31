#!/usr/bin/env bash
# Builds the release binary and writes dist/csr-<version>-linux-x86_64.tar.gz + dist/SHA256SUMS.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
bash acceptance/a1/cargo.sh build --release -p csr --locked
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' runner/Cargo.toml | head -1)"
NAME="csr-${VERSION}-linux-x86_64"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/$NAME" dist
install -m 0755 target/release/csr "$STAGE/$NAME/csr"
install -m 0644 config/csr.example.toml config/csr.service config/csr-update.path config/csr-update.service config/csr@.service config/csr-update@.path config/csr-update@.service "$STAGE/$NAME/"
install -m 0755 scripts/install-update.sh "$STAGE/$NAME/install-update.sh"
install -m 0644 README.md "$STAGE/$NAME/README.md"
install -m 0644 docs/install.md "$STAGE/$NAME/INSTALL.md"
[ -f LICENSE ] && install -m 0644 LICENSE "$STAGE/$NAME/LICENSE"
tar -C "$STAGE" --owner=0 --group=0 -czf "dist/$NAME.tar.gz" "$NAME"
(cd dist && sha256sum "$NAME.tar.gz" > SHA256SUMS)
echo "wrote dist/$NAME.tar.gz"
