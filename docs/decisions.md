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

## A1+ Phase 2: Network page
- Per-country view is an SVG bar list sorted by count (no map, no style attributes). The Overview ranking card calls `/api/network` only when `ranking.join` is true; the page polls every 30 s (server cache is 5 min).
- Monitor public API documented in the new `spec/monitor-public-api.md`.

## A2 Phase 1: sandbox and local runs
- `ckb-vm` is pinned to `=0.24.15` with the `asm` feature: it is the version in `Cargo.lock` of ckb v0.210.0 (`https://raw.githubusercontent.com/nervosnetwork/ckb/v0.210.0/Cargo.lock`). ISA `IMC|B|MOP`, VM version 2 as in ckb-script's `ScriptVersion::V2` (`data2`).
- The pinned ckb-vm has a fixed 4 MiB memory, so only `limits.max_vm_memory_mb = 4` / `--max-memory-mb 4` is accepted; other values make `run-local` fail with a CLI error (exit 1). Any out-of-bound memory access (including stack) maps to `memory_exceeded`.
- Wall time uses a watchdog thread that sets ckb-vm's `Pause` flag; the resulting `Error::Pause` maps to `timeout`.
- Status mapping (`runtime::classify`): VM errors other than cycles/memory/pause/invalid ecall/result-too-large (e.g. invalid instruction) are `script_error` with `exit_code` -1. Errors while loading the program map to `invalid_elf` when they are ELF parse errors. Cycles/result are only reported for `ok`/`script_error`; the `result_hex` is empty for the other statuses.
- Unknown syscall numbers: the host declines them and ckb-vm raises `InvalidEcall`, which maps to `unknown_syscall`. Unassigned numbers within 7001–7099 return 2.
- Syscalls cost 500 cycles each (as in CKB). Not provided: `load_transaction`, `exec`, `spawn` family (→ `unknown_syscall`); header syscalls answer index-out-of-bound because the virtual transaction has no header deps.
- Virtual transaction details: the job cell has capacity 0, an all-zero lock (`hash_type data`), empty data, out point (`ckb_hash("csr-virtual-job-cell")`, 0); the tx hash is `ckb_hash("csr-virtual-tx-v1" ‖ script_hash ‖ payload)`, so it is deterministic. Cell deps are the program (index 0) and the data cells.
- The service `code_hash` printed by `run-local` is the data hash of the identity file (default: the ELF); the virtual script's own `code_hash` is always the data hash of the program run in the VM.
- `run-local` creates `<data-root>/<network>/keys/` like `csr run`; it does not otherwise touch the data root yet.
- The Rust services workspace (`services/rust/`) is excluded from the root workspace; it uses `ckb-std =1.1.0` and a tiny local crate `csr-svc` for the raw `ecall`s of the service calls (`result_put`, `job_payload`). `hello-rust` reads witness 0 through ckb-std and returns via `result_put`.
- `scripts/build-services.sh` currently builds the Rust workspace (adds the riscv target through rustup if missing); C and JS follow in later phases.

## A2 Phase 2: service calls, storage, signing
- Storage uses `chacha20poly1305` + `hmac`/`sha2` (pure Rust). File name = HMAC-SHA256(storage key, "csr-storage-name-v1" ‖ key); the file name is the AEAD associated data. Usage is derived from file sizes (plaintext = size − 31 B overhead), the quotas are checked on every `kv_put` by listing the service directories.
- Return codes beyond the concept: storage/sign calls return 12 for an empty or over-long key (`kv_get`/`kv_delete`/`kv_mark_secret`/`delete_on_consume`); `kv_put` returns 11 for key > 256 B. `delete_on_consume` accepts a key that does not exist yet. Storage I/O or authentication errors stop the run as `script_error`.
- `sign` signs SHA-256 of the domain-prefixed message with the service's identity-file code hash (same as `code_hash` in the JSON). `simulate-consumed` applies rules of all services in the network's storage and prints `{"deleted":N}`.
- `dottest` was added too (needs only the calls of this phase). The Services registry and CLI come in a later phase.

## A2 Phase 4: C and JavaScript services
- **ckb-c-stdlib** is vendored (not a submodule) under `services/c/deps/ckb-c-stdlib/` from commit `6bf2bca668e0db5f9eb120eb78b31782899ef90d` (2026-08-12) without `tests/` and `simulator/`; see `services/c/deps/ckb-c-stdlib.PIN`. Vendoring keeps `build-services.sh` offline-capable.
- C flags follow ckb-c-stdlib's `guide.md`: `clang --target=riscv64 -march=rv64imc_zba_zbb_zbc_zbs -O3 -fdata-sections -ffunction-sections -fno-builtin-printf -fno-builtin-memcmp -nostdlib`, link `-static -Wl,--gc-sections` (plus `-Wall -Werror -Wl,--strip-all`). B extensions are fine: the VM runs with `ISA_B`.
- No system `ld.lld` is installed, so `build-services.sh` links C with the system `ld.lld` if present, else with the Rust toolchain's `rust-lld` symlinked as `.tools/bin/ld.lld` (lld picks its flavour from argv[0]).
- `hello-c` calls `job_payload` (7001) and `result_put` (7031) directly through ckb-c-stdlib's `syscall()` macro; the payload buffer is a static 65,530-byte array (results cannot be larger anyway).
- **ckb-js-vm**: the GitHub repository has no releases or tags, so the official binary is taken from the npm package `ckb-testtool@1.0.5`, which the ckb-js-vm monorepo publishes (`package/src/unittest/defaultScript/ckb-js-vm`). Its SHA-256 `721318de…a8c4` equals `build/ckb-js-vm` in the repo's reproducible-build `checksums.txt` at commit `29f3d1b3ee9e747ff3059c6046bda6bd0e2dafb9`. Committed as `services/vendor/ckb-js-vm`; `SHA256SUMS` records hash, URL and npm integrity, and `build-services.sh` verifies it.
- **Args layout** (from `qjs_load_cell_code_info` in `src/ckb_module.c`): `flags (u16 LE; bit 0 = file system) ‖ code_hash of the JS cell (32 B) ‖ hash_type (1 B) ‖ extra args`. ckb-js-vm finds the cell dep with `ckb_look_for_dep_with_hash2` (data hash via `load_cell_by_field` for data/data1/data2, type hash for `type`) and loads it with `load_cell_data`; the content may be JS source, QuickJS bytecode or a file system. `hello-js` uses `args = 0x0000 ‖ data_hash(hello.js) ‖ 04`, `data_cells = [hello.js]`, `identity = hello.js`, `elf = vendor/ckb-js-vm`. The hash is written into the manifest; `build-services.sh` fails if it no longer matches `hello.js`.
- **Payload/result in JS**: ckb-js-vm's `@ckb-js-std/bindings` module (built into the VM) offers the standard CKB syscalls but no raw syscall, so JS cannot call 7001–7099. `hello-js` reads the payload with `loadWitness(0, SOURCE_GROUP_INPUT)` and returns the result through the `CSR-RESULT:0x<hex>` debug line. Because a debug line holds at most 1 KB, this fallback carries at most 505 result bytes (the hex is cut off beyond that); larger JS results need a host extension in a later milestone.
- JS needs no bundling: `hello.js` is plain ES module source evaluated directly by ckb-js-vm, so no npm toolchain is installed. A run costs about 3.9 M cycles (QuickJS start-up), well below the default 70 M limit.

## A2 Phase 5: services registry
- Registry table `services(code_hash, name, language, status)` in the network's `csr.db`; `add-local` upserts name/language but keeps the status of a known code hash. Code hash is the identity-file data hash, stored as `0x…` lowercase.
- `approve` adds `0x<hash>` to `runtime.allowed_services` and `revoke` removes it (match ignores `0x`/case); a disabled service stays in the list (it is still approved; the status distinguishes it). `enable`/`disable` on a `not_approved` service fail (CLI exit 1, HTTP 409). Config is saved before the status; the CLI and the API share `registry::act`. Unknown code hash: HTTP 404 / CLI exit 1.
- The CLI service commands open `csr.db` directly (no running `csr run` needed); a running dashboard sees changes on its next request, but its in-memory config only learns about `allowed_services` changes made by the CLI after a restart.
- `storage_bytes` is the plaintext usage of the service directory; `jobs` and `earned_shannons` are 0 until A3.

## A3a Phase 1: contracts workspace and standards
- `contracts/` is its own cargo workspace (not in the root one). Besides the five contracts and `csr-contract-tests` it has a `csr-schemas` crate (no_std, generated molecule code from `contracts/schemas/csr.mol`, regenerated by `scripts/gen-schemas.sh` with `moleculec 0.9.2` installed under `.tools/`). The generated code is committed.
- `scripts/build-contracts.sh` passes the target, `-j 2` and the linker flag (`-s`) on the command line (`--config`), because `cargo test --manifest-path contracts/Cargo.toml` is run from the repo root, where `contracts/.cargo/config.toml` would not be read. Binaries are copied to `contracts/build/release/<name>`.
- Contracts are referenced in tests with hash type `data1` (deployed cells carry no Type ID).
- The system secp256k1 lock binary is `ckb-system-scripts 0.5.4` `specs/cells/secp256k1_blake160_sighash_all` (copied to `csr-contract-tests/fixtures/`, sha256 `32acace3…9d02f`); the harness signs with `ckb-crypto` (sighash_all message built by hand in `sign_tx`).
- Molecule `OnDemand` is an empty table; `network` and the attestation signature are `byte` / `byte[65]`.
- Error codes per script are listed in the three standards in `spec/`.
- The system lock also needs the `secp256k1_data` cell (fixture copied from the same crate), which `Env::verify*` adds as a cell dep.
