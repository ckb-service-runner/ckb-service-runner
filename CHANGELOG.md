# Changelog

## 0.1.0
- First Phase 1 release: release profile, `scripts/package.sh` (tarball + SHA256SUMS), `config/csr.service`, `docs/install.md`, greyed-out runtime toggle on mainnet; `docs/milestones/A4.md`.

## Unreleased
- A8b phase 1: `csr-dev attest` no longer has a devnet faucet (`--faucet-ckb`, the transfer, the `faucet` JSON key and the stderr line are gone); the A8 tests that needed it are replaced by `acceptance/a8b/p3`–`p5` with 700 CKB; setup figures in the docs corrected (about 700 CKB to send, registry cell about 540 CKB). See `docs/milestones/A8b.md`.
- A8: the runner wallet (D-A8-1). Setup needs one transfer to the runner address and one signed message; tips collect in the runner wallet and are swept to `[payout] sweep_address`. Dashboard page "Wallet & registration" (`#/wallet`; runner address with copy button, balance, send form, sweep settings, text to sign, signature, Register/Update stake, Remove with in-page confirmation), Overview card "Wallet", Settings `payout.sweep_address|sweep_threshold_ckb|sweep_amount_ckb`. **Removed:** the anyone-can-pay payout cell and `csr payout …`, `[payout] lock_args`, `csr register prepare`, `/api/payout…`, `/api/registry/prepare-create`, `/api/tx/submit-signed` and their dashboard controls (`csr tx submit` stays); acceptance/a7 p2–p5 are obsolete. See `docs/milestones/A8.md`.
- A8 phase 2 (D-A8-1): the runner wallet. `csr wallet status|send|sweep`, `GET /api/wallet`, `POST /api/wallet/send`; automatic sweep in `csr run` (`[payout] sweep_address`, `sweep_threshold_ckb`, `sweep_amount_ckb`; `lock_args` ignored); `runner_address` in `csr identity` and `/api/runner-cell`. Registration owned and paid by the runner wallet with the stake owner's signed consent: `csr register consent-message|verify-consent|create|remove`, `POST /api/registry/consent-message|create|remove`. Tips go into the runner wallet (`/api/earnings` reports `wallet`). Removed: `csr payout …`, `csr register prepare`, `/api/payout…`, `/api/registry/prepare-create`, `/api/tx/submit-signed`. `csr-dev sign-message`, `reclaim`, `deploy-contracts --reuse-unchanged`; `csr-dev attest` sends a devnet faucet of `--faucet-ckb` (default 200) CKB to the runner wallet.
- A7: dashboard page "Stake & payout" (`#/payout`) replaces the Float page, Overview card "Payout", setting `payout.lock_args`, stake of the CSR monitor in "My rank" (`node.stake` in `/api/network`); `spec/float-lock.md` → `spec/dual-lock.md`, job/monitor specs, testing runbook and `docs/milestones/A7.md` updated for the DAO stake and payout cell.
- A7 phase 3: registration with the DAO stake: `csr register prepare --stake 0x<tx>:<index>` (required; checks payout cell and stake, stake = `cell_deps[0]`, registry data v2), `csr register status` reports `stake {outpoint, status, capacity_shannons}`, `csr register attest --stake` sends the stake to `POST /v1/attest` and shows the monitor's `reason`; `POST /api/registry/prepare-create` takes `stake`; `POST /v1/attest` with `stake_invalid`/`stake_in_use` documented in `spec/ping-protocol.md` §5a and `spec/monitor-public-api.md`.
- A7 phase 2: payout cell replaces the float: `[payout] lock_args`, `csr payout status|prepare-create|prepare-withdraw`, `GET /api/payout`, `POST /api/payout/prepare-create|prepare-withdraw`; `csr float` and `/api/float…` removed. Deployments: `scripts.anyone_can_pay` (with `dep_type`) replaces `float_lock`; vendored `devtools/vendor/anyone_can_pay` (ckb-sdk 3.7.0, pinned in `SHA256SUMS`). `csr-dev`: `deploy-contracts` without the float, with ACP and `--keep-config`; new `dao-deposit`; `sign-tx` signs every secp256k1/ACP lock group of the key.
- A7 phase 1 (D-A7-1): Registry v2 (`payout_lock_hash`, `stake` = Nervos DAO deposit out point); `csr-registry` checks the stake deposit (errors 18–21, `MIN_STAKE` 10,000 CKB); `csr-job-lock` pays tips to the registered payout cell; `csr-float-lock` and its tests removed; `spec/registry.md` rewritten for v2.
- A6: dashboard "Network" tab becomes "CSR monitor" with a world map of the CSR network (`GET /v1/locations`, shared `worldmap.js`, 2° dots); Overview card "Ranking" and setting "Join ranking" renamed; `docs/milestones/A6.md`.
- A5: testnet registration: chain commands work on testnet with the embedded `deployments/testnet.json`, real testnet monitor defaults, and `csr register attest --out FILE` fetches and verifies the monitor attestation; `docs/milestones/A5.md`.
- A3c phase 3: race between two runners on one devnet node (loser → `Lost` on pool double-spend errors), dashboard Earnings & float page and Overview float card; `docs/milestones/A3c.md`, `docs/testing.md`, README.
- A3c phase 2: keeper execution (`runner/src/keeper.rs`): runs persisted in SQLite (`job_runs`, `job_events`, `settlements`) with the full lifecycle, sandbox execution of on-chain service code, run transaction (runner key cell, job with trigger `since`, float + tip, recreation/refund, registry + monitor config deps, fee from the job), chained batches, settlement tracking (`Settled`/`Lost`/`Rejected`); host calls `chain_read` (7003) and `cells_query` (7004); `GET /api/jobs`, `GET /api/jobs/{id}`, `GET /api/earnings`; dashboard Jobs page.
- A3b phase 4: `csr register prepare|status`, `POST /api/registry/prepare-create`, `GET /api/registry`, dashboard Register button; devnet operator flow and testnet runbook in `docs/testing.md`; `docs/milestones/A3b.md`.
- A3b phase 2: `csr-dev` (deploy-contracts, clock-update, attest, fund, sign-tx, owner-lock-args); `csr` accepts `network = "devnet"` + `[devnet]` in debug builds (R2 without peer rule); `csr identity --json`.
- A3a security fixes, phase 2: `csr-job-lock` run path requires the registered node's runner key cell (F2, error 15) and a live attestation by the monitor config cell dep (F5, errors 16/17); job tests and helpers updated (config dep in every run); F6–F8 documented; job cycles re-measured; downstream impact in `docs/decisions.md`.
- A3a security fixes, phase 1: `csr-registry` requires the runner key cell on creation/key change (F1, error 17); `csr-float-lock` runner path forbids type/data on the float output (F3, error 10) and any capacity loss (F4; `max_fee_shannons` reserved).
- A3a phase 4: `csr-job-type` (creation funding, run counts, Every update) and `csr-job-lock` (triggers via `since`, registered-float tip, anti-siphon, refund path) with 42 ckb-testtool tests; job, float and registry standards updated; `docs/milestones/A3a.md`.
- A3a phases 1–3: contracts workspace (`contracts/`), molecule schemas, `scripts/build-contracts.sh`, ckb-testtool harness, `csr-float-lock`, `csr-dual-lock`, `csr-registry` (in-script secp256k1 attestation check), `spec/float-lock.md`, `spec/registry.md`, `spec/job-cell-standard.md`.
- A2 phase 5: services registry (SQLite), `csr service add-local|list|approve|revoke|enable|disable`, `GET /api/services`, `POST /api/services/{code_hash}/{approve|revoke|enable|disable}`, dashboard Services page, `docs/milestones/A2.md`. **Milestone A2 complete.**
- A2 phase 4: C (`hello-c`, vendored ckb-c-stdlib) and JavaScript (`hello-js` on the vendored ckb-js-vm) example services; `build-services.sh` builds C and verifies the JS artefacts.
- Add `GET /api/network` and the monitor public-API client with a 5-minute cache (A1+ phase 1).
- A1+ phase 2: dashboard Network page (`#/network`), Overview ranking card with rank and 30-day availability, `docs/milestones/A1plus.md`, `spec/monitor-public-api.md`.

## A1 phase 1
- Workspace, `csr version`, `csr config validate`, full config model and validation, example config, network defaults.

- Phase 2: node JSON-RPC client, R2 guard, `csr check` and `csr detect`.

## A1 phase 3
- `csr run`: service core with graceful shutdown, startup refusals, mainnet confirmation, key generation, SQLite state, file logging, R2 guard loop, resource monitor with hold conditions, and the read API (`/healthz`, `/api/status`, `/api/node`, `/api/resources`, `/api/resources/history`, `/api/logs`).

## A1 phase 4
- Dashboard sessions, login and CSRF protection; `csr set-password` (argon2id); `GET/PUT /api/settings` with deep merge, validation, atomic save with `.bak`, live log level; `GET /api/events` SSE.

## Phase 5
- Embedded dashboard (Overview, Node, Resources, Settings, Logs), CSP + nosniff headers.

## Phase 6
- Ping protocol v1 spec (`spec/ping-protocol.md`), binding/answer signatures and the inbound `POST /v1/challenge` responder with rate limit and replay protection.

## A1 phase 7
- Outbound check-in (`ping_mode` outbound/both): `/v1/checkin` → validated challenge → `/v1/answer`, every 10 min (2 s with fast tick), `last_check_ok_at` on acceptance.
- Rust mock monitor and ping-flow integration tests; README, `docs/milestones/A1.md`, open questions. **Milestone A1 complete.**
- A3b Phase 3: `csr runner-cell status`, `csr float prepare-*|status`, `csr tx submit`, matching HTTP routes and a dashboard Float page.
