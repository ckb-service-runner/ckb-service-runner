# Milestone A3, part b – CSR on the chain

## Built
- `scripts/devnet.sh` (start, mine, status, stop, reset): pinned `ckb` v0.210.0 devnet on 127.0.0.1:18114/18115.
- `csr-dev` (`devtools/csr-dev`): deploy-contracts, clock-update, attest, fund, sign-tx, owner-lock-args; plays monitor and wallet on devnet.
- `csr` on devnet (debug builds only): `network = "devnet"`, `[devnet] deployments`, R2 without the peer rule.
- Runner key cell (`csr identity`, `csr runner-cell status`), float cell (`csr float prepare-create|prepare-topup|prepare-withdraw|status`), registry cell (`csr register prepare|status`), `csr tx submit`.
- HTTP API: `/api/runner-cell`, `/api/float`, `/api/float/prepare-*`, `/api/registry`, `/api/registry/prepare-create`, `/api/tx/submit-signed`; dashboard Float page with Create / Top up / Withdraw / Register and signed-file upload.

## Registration
`csr register prepare` builds the registry creation transaction: inputs `[runner key cell, owner cells]`, outputs `[registry cell (dual lock, registry type, molecule `Registry` data with the attestation), runner key cell (identical), owner change]`, cell deps secp256k1, registry code, monitor config cell. The owner pays capacity and fee; CSR signs the runner key cell input with the service key (registry rule 6 / F1). Refusals: R2 not Synced, no float, no runner key cell, attestation for another key or network, expired attestation.

## Demo
```
bash scripts/build-contracts.sh && bash scripts/devnet.sh start
node --test --test-concurrency=1 acceptance/a3b/p2.test.mjs acceptance/a3b/p3.test.mjs acceptance/a3b/p4.test.mjs
```
See `docs/testing.md` for the manual flow and the testnet runbook.

## Not done
Job execution (part c); shipped testnet/mainnet script hashes; verified Neuron offline-signing import (`docs/open-questions.md`).
