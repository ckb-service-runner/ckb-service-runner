# Changelog

## Unreleased
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
