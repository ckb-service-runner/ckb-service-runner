#!/usr/bin/env bash
# Devnet versions of the manual checks of the Oct 2026 audit (specs 03, 04, 09, 17):
# skeleton guard, delete-on-consume, attestation renewal, cell-consumed trigger.
# Builds what is needed, starts a fresh devnet, runs acceptance/manual-checks/devnet.test.mjs,
# stops the devnet and prints PASS / FAIL per check. Touches only the devnet ports (18114/18115).
set -uo pipefail
cd "$(dirname "$0")/.."
LOG=${LOG:-manual-checks.log}
echo "== build (contracts, services, csr, csr-dev)"
bash scripts/build-contracts.sh >/dev/null || { echo "contract build failed"; exit 1; }
bash scripts/build-services.sh >/dev/null || { echo "service build failed"; exit 1; }
cargo build -q || { echo "cargo build failed"; exit 1; }
echo "== devnet"
bash scripts/devnet.sh reset >/dev/null && bash scripts/devnet.sh start >/dev/null || { echo "devnet did not start"; exit 1; }
echo "== checks (about 5 minutes; full output in $LOG)"
node --test --test-concurrency=1 acceptance/manual-checks/devnet.test.mjs >"$LOG" 2>&1
rc=$?
bash scripts/devnet.sh stop >/dev/null
grep -E '^(ok|not ok) ' "$LOG" | sed -E 's/^ok [0-9]+ - /PASS  /; s/^not ok [0-9]+ - /FAIL  /'
grep -E '^# (pass|fail) ' "$LOG"
exit $rc
