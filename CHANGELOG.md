# Changelog

## A1 phase 1
- Workspace, `csr version`, `csr config validate`, full config model and validation, example config, network defaults.

- Phase 2: node JSON-RPC client, R2 guard, `csr check` and `csr detect`.

## A1 phase 3
- `csr run`: service core with graceful shutdown, startup refusals, mainnet confirmation, key generation, SQLite state, file logging, R2 guard loop, resource monitor with hold conditions, and the read API (`/healthz`, `/api/status`, `/api/node`, `/api/resources`, `/api/resources/history`, `/api/logs`).

## A1 phase 4
- Dashboard sessions, login and CSRF protection; `csr set-password` (argon2id); `GET/PUT /api/settings` with deep merge, validation, atomic save with `.bak`, live log level; `GET /api/events` SSE.
