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
