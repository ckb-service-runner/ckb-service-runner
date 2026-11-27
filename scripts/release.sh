#!/usr/bin/env bash
# Builds a release: dist/csr-<version>-<arch>-linux (the asset the dashboard update downloads), the
# tarball of scripts/package.sh, and dist/SHA256SUMS over both. Prints the command that signs
# SHA256SUMS offline. The signing key never enters the repository: this script reads no key.
# Usage: scripts/release.sh <version> [--ci-dir <dir>]
#   <version> must equal the version in runner/Cargo.toml.
#   --ci-dir: do not build; take the files built by the release-build workflow from <dir>.
set -euo pipefail
USAGE="usage: scripts/release.sh <version> [--ci-dir <dir>]"
VERSION="${1:?$USAGE}"
VERSION="${VERSION#v}"
shift
CI_DIR=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ci-dir)
      [ $# -ge 2 ] || { echo "--ci-dir needs a directory ($USAGE)" >&2; exit 2; }
      CI_DIR="$2"; shift 2 ;;
    *) echo "unknown argument: $1 ($USAGE)" >&2; exit 2 ;;
  esac
done
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' runner/Cargo.toml | head -1)"
if [ "$VERSION" != "$CARGO_VERSION" ]; then
  echo "version $VERSION does not match runner/Cargo.toml ($CARGO_VERSION)" >&2
  exit 1
fi
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "version must be X.Y.Z (the dashboard update offers no pre-releases)" >&2
  exit 1
fi
ARCH="$(uname -m)"

ASSET="csr-${VERSION}-${ARCH}-linux"
TARBALL="csr-${VERSION}-linux-x86_64.tar.gz"
if [ -n "$CI_DIR" ]; then
  for f in "$ASSET" "$TARBALL"; do
    if [ ! -f "$CI_DIR/$f" ]; then
      echo "missing $f in --ci-dir $CI_DIR (download the release-build artifacts first, see docs/release.md)" >&2
      exit 1
    fi
  done
  mkdir -p dist
  install -m 0755 "$CI_DIR/$ASSET" "dist/$ASSET"
  install -m 0644 "$CI_DIR/$TARBALL" "dist/$TARBALL"
else
  bash scripts/package.sh
  install -m 0755 target/release/csr "dist/$ASSET"
fi
FILES=("$ASSET" "$TARBALL")
WINZIP="csr-${VERSION}-x86_64-windows.zip"
if [ -f "dist/$WINZIP" ]; then FILES+=("$WINZIP"); fi
(cd dist && sha256sum "${FILES[@]}" > SHA256SUMS)
rm -f dist/SHA256SUMS.sig

cat <<MSG
wrote dist/$ASSET, dist/$TARBALL and dist/SHA256SUMS:
$(cat dist/SHA256SUMS)

Sign SHA256SUMS offline (on the machine that holds the release key, never in this repository):

  openssl pkeyutl -sign -rawin -inkey <key file> -in SHA256SUMS -out SHA256SUMS.sig

then check it with the public key and upload $ASSET, $TARBALL${WINZIP:+ (and $WINZIP if listed above)}, SHA256SUMS and SHA256SUMS.sig
to the GitHub release v$VERSION (see docs/release.md).

Build provenance (for files built by the release-build workflow), check each file with:
$(for f in "${FILES[@]}"; do echo "  gh attestation verify dist/$f --repo ckb-service-runner/ckb-service-runner"; done)
MSG
