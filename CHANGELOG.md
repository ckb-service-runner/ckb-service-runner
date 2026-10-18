# Changelog

## Unreleased
- Add `GET /api/network` and the monitor public-API client with a 5-minute cache (A1+ phase 1).

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
