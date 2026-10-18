# Decisions

- Phase 1: `docs/spec/` was not present in the repo; implemented from the concept text only.
- Config fields are plain strings/numbers validated in `Config::validate` (not enums), so the error messages stay readable and the settings API can reuse the same struct later.
- `dashboard.bind` and `ranking.ping_bind` must be `ip:port` socket addresses (hostnames are rejected).
- `--network` is restricted by clap to `testnet|mainnet`; clap's default `--version` flag is disabled in favour of the `version` subcommand.
- Network defaults live in `config/networks/<net>.toml` with keys `monitor_url` / `monitor_pubkey` (placeholders, TODO).
- Phase 1 (retry): the Rust toolchain was not actually installed for this user (`cargo: not found`); installed it with rustup (minimal profile + rustfmt + clippy) into `$HOME/.cargo`, the location the environment description assumes. Dead-code lint is silenced in `network.rs` until later phases use it.
- Phase 2: node RPC is a small `reqwest` client (5 s timeout, proxies disabled, no batching); calls are sequential and stop at the first transport error. Any non-transport RPC error on `get_blockchain_info`/`sync_state` counts as "unsupported"; any non-"method not found" error on `get_indexer_tip` counts as "indexer tip unavailable".
- R2 evaluation is a pure function (`node::r2::evaluate`) over a `Snapshot`, so the state machine is unit-tested without sockets. A failed `get_peers` counts as 0 outbound peers.
- `detect --json` reports `node_type: null` and `indexer_enabled: null` when unreachable; `--probe` adds an extra `probes` array (not used by tests). `hint` is also set for unreachable/unsupported nodes.
- `check` exit codes: 0 only for Synced, 2 otherwise, 1 for config errors (nothing on stdout).
- Phase 3: `csr run` uses a single-threaded tokio runtime. Startup order: config → mainnet confirmation check → first node poll (refusals on chain mismatch / light client) → only then data directories, marker, logging, keys and DB are created, so a refused start leaves no files. The mainnet marker is written right after these checks pass. If the node is unreachable at start, the same first-contact check runs on the first successful poll and, on refusal, stops the service with exit 1.
- R2 guard tick = 7 JSON-RPC calls (the Phase 2 snapshot plus `tx_pool_info`) every 10 s, i.e. 0.7 calls/s. The first tick uses the startup poll; the loop's first tick comes one period later.
- `/api/node.epoch` is the epoch number (low 24 bits of the packed header epoch), as a JSON number. `/api/node` fields are `null` while the node is unreachable.
- `/api/resources` percentages are of the whole machine (per-process CPU divided by core count); `ckb_percent` sums processes named exactly `ckb`. The resource sample refreshes every 5 s (1 s with fast tick) and is also seeded before the server starts; samples go to SQLite every 60 s (1 s fast) and are pruned after 24 h.
- Logging: own `Tee` writer to `<net>/logs/csr.log` + stderr (no tracing-appender, so lines are on disk immediately); the file is rotated once to `csr.log.1` at startup above 10 MB. Only the `csr` target logs at the configured level, dependencies at `warn`. `/api/logs` reads the last 512 KB and filters by the level token of each line; an unknown level gives 400. The level can be changed live through `AppState::set_log_level` (used by the settings API later).
- Runtime state `paused` currently only reflects R2 != Synced; resource hold conditions (`resources::hold_reason`) are for the job scheduler later.
- Unit tests cannot use `CARGO_TARGET_TMPDIR` (integration tests only); they use `target/unit-tmp`.
- Phase 4: sessions are random 256-bit tokens in an in-memory map (24 h TTL, max 1000). A successful login issues a fresh session id and CSRF token (no session fixation); a failed login waits 500 ms. Auth check comes before the CSRF check (password set + unauthenticated → 401; any write without valid cookie+token → 403). `/api/session` always works; `authenticated` is true when no password is set.
- Settings are saved by re-serialising the whole config with `toml` (comments in the file are lost; the `.bak` keeps the previous version). The file is written 0600 via a temp file + rename. `PUT /api/settings` rejects `dashboard.password_set` and `dashboard.password_hash` (400); the password is only changed with `csr set-password`. A `--network` CLI override is persisted if settings are saved while it is active.
- `csr set-password` reads the file without validating (so a public bind can get its password), validates afterwards, and prompts twice with `stty -echo` when no `--password-stdin` is given. `restart_required` compares against the config currently in memory.
- SSE `status` events: sent on connect, when the status (ignoring `uptime_s`) changes (checked every 500 ms) and at least every 10 s.

## Phase 5: dashboard
- Static assets are embedded with `rust-embed` and served by the router fallback; CSP and `X-Content-Type-Options` are added to every response by a middleware layer.
- Dashboard JS uses SSE with a 10 s polling fallback; theme is stored in localStorage.

## Phase 6: inbound ping responder
- Ping protocol is documented in `spec/ping-protocol.md`. Signing uses `secp256k1` + `sha2` (SHA-256 of the UTF-8 message); S is normalised before verifying monitor signatures; our signatures are low-S (libsecp256k1 default).
- The ping listener is a separate axum server on `ranking.ping_bind`, started at `csr run` only when `ranking.join` and mode is `inbound`/`both`; it is not re-evaluated on live settings changes (restart needed). Body limit 4 KB (413), rate limit 10 per rolling 60 s per client IP counting every request (429).
- Reject codes: 400 malformed/stale/future/height above tip/bad nonce, 403 bad monitor signature, 409 replayed nonce, 503 node data unavailable. The nonce is reserved only after signature, freshness and height checks pass and is released if the answer cannot be built; replay window 10 min is kept in memory (`AppState::ping_nonces`), so it is lost on restart.
- Challenge JSON rejects unknown fields. `last_check_ok_at` is set when an answer is served.

## Phase 7
- The outbound loop always runs and re-reads `ranking.join` / `ping_mode` each tick, so settings changes apply live; the first check-in happens at startup, right after the initial R2 poll.
- A failed answer POST releases the nonce reservation; a non-200 reply is a failure retried at the next interval.
- Spec `docs/spec/` is absent in this checkout; the concept text was used as the source of truth.

## A1+ Phase 1: Network API
- `runner/src/monitor/` fetches `/v1/network` and `/v1/nodes/{peer_id}` lazily on the first `GET /api/network` after the 5-minute cache expires (no background refresh). Both endpoints are cached together as one snapshot keyed by (monitor URL, peer ID); a changed key invalidates it.
- A 404 for the node is a successful "not ranked" (`node: null`). Any other failure of either request fails the whole refresh: cached data is served with `stale: true` + `error`; with no cache `network`/`node` are `null` and `error` is set (HTTP 200).
- After a failed refresh the monitor is not asked again for 15 s (avoids hammering on every dashboard poll). Requests: 10 s timeout, no proxy, no redirects. Unknown reply fields are ignored.
- If the node's peer ID is not known yet, only `/v1/network` is fetched and `node` is `null`.
