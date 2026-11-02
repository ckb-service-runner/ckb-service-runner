#!/usr/bin/env bash
# Builds dist/csr-<version>-x86_64-windows.zip (Windows beta) from a csr.exe built by the windows.yml workflow:
# csr.exe, csr.example.toml, networks\, install.cmd, install.ps1, uninstall.cmd, uninstall.ps1, LICENSE,
# README-windows.txt.
# Contains no key, no other .toml and no data. Prints the zip path.
# Usage: scripts/package-windows.sh <version> <path-to-csr.exe>
set -euo pipefail
VERSION="${1:?usage: scripts/package-windows.sh <version> <path-to-csr.exe>}"
EXE="${2:?usage: scripts/package-windows.sh <version> <path-to-csr.exe>}"
VERSION="${VERSION#v}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
[ -f "$EXE" ] || { echo "csr.exe not found: $EXE" >&2; exit 1; }
EXE="$(cd "$(dirname "$EXE")" && pwd)/$(basename "$EXE")"

NAME="csr-${VERSION}-x86_64-windows"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
PKG="$STAGE/$NAME"
mkdir -p "$PKG/networks" dist
install -m 0644 "$EXE" "$PKG/csr.exe"
install -m 0644 config/csr.example.toml "$PKG/csr.example.toml"
for f in config/networks/*.toml; do install -m 0644 "$f" "$PKG/networks/"; done
install -m 0644 scripts/install.ps1 scripts/uninstall.ps1 scripts/install.cmd scripts/uninstall.cmd "$PKG/"
[ -f LICENSE ] && install -m 0644 LICENSE "$PKG/LICENSE"
cat > "$PKG/README-windows.txt" <<TXT
CSR (CKB Service Runner) $VERSION for Windows - BETA
=====================================================

CSR runs beside your CKB full node (ckb.exe, or Neuron's internal mainnet node) and gives you
a dashboard and a place in the CSR Network Monitor ranking. x86_64 only, unsigned, no auto-update.

You need a CKB FULL node on this PC: ckb.exe, or Neuron's internal MAINNET node (Neuron on testnet
runs a light client, which CSR cannot use). It may still be syncing.

Install: unzip the whole folder, then double-click install.cmd and allow the Administrator prompt.
         Windows may warn that the files come from the internet (the beta is unsigned): choose
         "More info" > "Run anyway". The installer finds your node, sets everything up and opens
         the dashboard in your browser. A Start menu folder "CSR" links to the dashboard.

Options (-Rpc, -CkbDir, -NoRanking, -NoBrowser): open PowerShell as Administrator in this folder:
  Set-ExecutionPolicy -Scope Process Bypass
  .\install.ps1 -CkbDir <folder of your node>

To update, run install.cmd of the new zip again (configs, keys and data are kept).
To remove: double-click C:\Program Files\csr\uninstall.cmd (data is kept; "uninstall.cmd -RemoveData"
from a command prompt deletes it too).

Dashboards: http://127.0.0.1:8130 (testnet), http://127.0.0.1:8140 (mainnet).
Full guide: docs/windows.md in https://github.com/ckb-service-runner/ckb-service-runner
TXT

ZIP="$ROOT/dist/$NAME.zip"
rm -f "$ZIP"
if command -v zip >/dev/null 2>&1; then
  (cd "$STAGE" && zip -q -r "$ZIP" "$NAME")
else
  python3 -I - "$STAGE" "$NAME" "$ZIP" <<'PY'
import os, sys, zipfile
stage, name, out = sys.argv[1:4]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for d, _, files in os.walk(os.path.join(stage, name)):
        for f in sorted(files):
            p = os.path.join(d, f)
            z.write(p, os.path.relpath(p, stage).replace(os.sep, "/"))
PY
fi
echo "$ZIP"
