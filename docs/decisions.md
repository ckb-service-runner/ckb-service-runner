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
