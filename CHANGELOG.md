# Changelog

## 0.4.1
- CSR monitor map: dots at 60 % of their size; your own node's dot is larger and orange, with a "your node" key under the map and in its label. Needs a monitor whose `/v1/nodes` sends `location` (older monitors: no own dot, nothing else changes).
- API: `/api/network` passes on the monitor's `node.location` (`{lat, lon}`, the node's 2° map cell, or `null`).

## 0.4.0
- New dashboard look (ckbdash style) with a light and a dark theme (the theme button stays in the header).
- Overview: node card (tip, behind, tip age, peers, sync bar, epoch, explorer link), CSR monitor card with rank, score and its terms, Wallet & registration card (balance, stake, attestation expiry, next renewal), service runtime card (job counts, tips; the Runtime switch now saves the setting), last 24 hours sparklines, machine bars, CSR network card, update card. A warning card when the attestation expires within 3 days without automatic renewal.
- Node page: the node card with tip hash and node ID, node identity, tip lag history, peers and tx pool.
- Auto renew (`[registry] auto_renew`, on by default): the attestation is renewed 2 days before it expires whether the service runtime is on or off (before: only with the runtime on). Switch and renewal timeline on the Wallet & registration page; also in Settings.
- API: `/api/status` has `hostname` and `links` (explorer and node list URLs come from the runner, the assets keep no external URLs); `/api/node` has `epoch_index`, `epoch_length`, `tip_timestamp_ms`, `node_id`, `rpc_url`; `/api/registry` has `auto_renew` and `next_renewal_ms`; `/api/network` passes on the monitor's `score` and `metrics`.

## 0.3.1
- Dashboard: elements marked `hidden` stay hidden (the "Remove the registration? Yes, remove / Cancel" confirmation was always visible because `.row` set `display:flex`).
- A new registration (`csr register create`, dashboard "Register") clears the error of an earlier attestation renewal.
- "Check for update" asks GitHub again (cache at most 60 s instead of 1 hour); the daily background check keeps the hour.

## 0.3.0
- Contracts v3 on testnet (`deployments/testnet.json`, new config cell `0x912124d6…`): Registry v3, CSR-ATTEST-v2 (stake and registry type hash signed), `csr-config-type`, 8-day attestation cap, refund at the job input's index; runners register again (`csr register create`). Docs follow the code (csr-docs). Devnet tests and the manual checks follow v3.
- Contracts v3 phase 2: the runner writes and reads Registry v3 (`chain::RegistryV3` encode/decode; `parse_registry_data` still reads v1/v2 cells), runner-owned registry cells with 32-byte dual-lock args, `CSR-ATTEST-v2` (`attest::attestation_message_v2`; `check` and `register create` verify stake and registry type hash, the 8-day cap and the config cell's monitor key), new setting `ranking.public_endpoint` written into the cell (`endpoint`), `register status` shows `registry_version` and `endpoint`; deployments files with `scripts.config_type` (v3; the config cell found under `csr-config-type`, registry args = `config_id`, hashes checked); `build_run` keeps the job output at the job input's index (`JOB_INDEX`). `csr-dev`: `deploy-contracts` deploys `csr-config-type` and a new config cell of that type (`--keep-config` only for such a cell), `clock-update` never moves the clock back, `attest --stake` signs v2. Docs: devnet flow on v3 (`docs/testing.md`), testnet migration (`docs/install.md`).
- Contracts v3 phase 1 (D20 A–D24 A): `RegistryV3` (node ID once, optional `endpoint`, `version = 3`) and `CSR-ATTEST-v2` (signs stake and registry type hash) in `csr.mol`, `csr-registry` and `csr-job-lock`; 8-day attestation lifetime cap (registry error 24, job lock 18); consent of the stake owner for `peer_id`/`network`/`service_pubkey` changes; 32-byte dual-lock args for runner-owned cells; new `csr-config-type` (Type ID, clock only forward, monitor key fixed, no destroy) on the monitor config cell, found by its type hash (registry args = its Type ID); job refund/recreation at the job input's index (job lock error 19). Specs and `docs/decisions.md` updated. Runner and `csr-dev` follow in later phases.

## 0.2.1
- First update delivered through the signed update path (dashboard "Update"); the dashboard session helper `anonymous_count` is test-only (no dead-code warning in release builds).

## 0.2.0
- First signed release: `RELEASE_PUBKEY` is set (ed25519, private key offline), so the dashboard "Update" and `csr update stage` accept releases signed with it; `docs/release.md`.
- Audit fixes, Oct 2026 (PR #1): the AIWorker branches before contracts v3 (skeleton guard, delete-on-consume, keeper robustness, dashboard security, sandbox storage v2, runner fixes, attestation renewal, check-in v1.1, node identity, cell-consumed trigger, update check) and the integration fixes; a spent cell counts as consumed when the node answers `unknown`; devnet manual checks `scripts/manual-checks.sh`.
- csr-update-check phase 1 (D5 B): update check and verified staging. `csr update check|stage|verify`, `GET /api/update`, `POST /api/update/install` (needs a dashboard password), daily background check with a notice and "Check for update"/"Update" on the Overview; ed25519-signed `SHA256SUMS` checked against `RELEASE_PUBKEY` (all zeros in this build: every update is refused); root installer `config/csr-update.path`, `config/csr-update.service`, `scripts/install-update.sh` (verify by the installed binary, `csr.prev` rollback); `scripts/release.sh`, `docs/release.md`; `csr --version`.
- A8b phase 1: `csr-dev attest` no longer has a devnet faucet (`--faucet-ckb`, the transfer, the `faucet` JSON key and the stderr line are gone); the A8 tests that needed it are replaced by `acceptance/a8b/p3`–`p5` with 700 CKB; setup figures in the docs corrected (about 700 CKB to send, registry cell about 540 CKB). See `docs/milestones/A8b.md`.
- A8b phase 2: dashboard – setup text (700/540 CKB), runner address shown in full without a Copy button, Copy of the text to sign works over plain http (execCommand fallback), project logo (header, login, favicon), PNG served as `image/png`.
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

## 0.1.0
- First Phase 1 release: release profile, `scripts/package.sh` (tarball + SHA256SUMS), `config/csr.service`, `docs/install.md`, greyed-out runtime toggle on mainnet; `docs/milestones/A4.md`.
