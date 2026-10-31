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

## A3a Phase 2 – float lock
- `csr-float-lock` depends on `ckb-std` **without** the `ckb-types` feature and parses the Script molecule
  (args offset, output lock comparison by raw bytes) by hand. The `bytes` crate pulled in by `ckb-types` emits
  RISC-V atomic (AMO) instructions that ckb-vm rejects (`InvalidInstruction`); `dummy-atomic` alone does not cover it.
  Later contracts that need molecule types must avoid `bytes` the same way (or build with `-C target-feature=-a`
  and a `bytes`-free dependency tree).
- The harness test now spends the float lock with an owner key cell, since the real lock rejects a spend without a key cell.
- Float-lock tests assert the failing script (`Inputs[0].Lock`) and the error code from `spec/float-lock.md`.

## A3a Phase 3 – registry and dual lock
- Contracts that run in a separate cargo invocation do not get `ckb-std`'s `ckb-types` feature through feature
  unification, so `csr-dual-lock` and `csr-registry` use only `ckb_std::syscalls` (no `high_level`). The registry
  does not use `csr-schemas` (molecule pulls the `bytes` crate) and parses the molecule data by hand
  (`csr-registry/src/mol.rs`, strict field counts).
- secp256k1 recovery in-script: pure-Rust `k256 0.13` (`ecdsa` feature, no defaults) + `blake2b-ref 0.3`.
  ~7.9 M cycles per registry creation for the type script (details in `spec/registry.md` §7). Low-S is required.
- "Output lock is `csr-dual-lock`": the registry args are fixed to `config_type_hash` by the concept, so the dual
  lock's code hash is **embedded at build time**: `csr-registry/build.rs` hashes
  `contracts/build/release/csr-dual-lock`; `scripts/build-contracts.sh` therefore builds the dual lock first, then
  the other contracts. Accepted hash types: `data1`, `data2`. A changed dual lock means a new registry binary.
- Runner lock check accepts the sighash lock as genesis type hash `0x9bd7e06f…` (`type`) or as data hash
  `0x709f3fda…` (`data`/`data1`/`data2`); tests use `data1` with the fixture, a test pins the fixture hash.
- On update/destroy the owner hash comes from the input's dual-lock args. Runner-path checks (fields, lock,
  capacity) run before the attestation checks, so a forbidden change reports 14/15 even if the attestation is
  also bad.
- `network > 2` in registry data is rejected as malformed (error 7).
- The harness test now creates a real registry cell (valid attestation) instead of the placeholder spend.
- "Registry without attestation" is tested as an attestation with an all-zero signature (error 10); missing
  fields would be error 7.

## A3a Phase 4 – job type and job lock
- **Fee from the job, tip to the float** (concept, replaces spec §3.3 "float pays the fee, job pays it back"):
  same economics with fewer moving parts. The job cell loses at most `tip + max_fee` per run; the float gains
  at least the tip (net), its own `max_fee_shannons` bound is not used by the run.
- `csr-job-type` and `csr-job-lock` drop `csr-schemas` and the `ckb-types` feature like the registry; they share
  `csr-registry/src/mol.rs` and `csr-job-type/src/{job,util}.rs` via `#[path]` includes (no new crate, so
  `Cargo.lock` only lost entries). Removing the job crates' `ckb-types` feature also removed it from the float
  lock's build (feature unification), so `csr-float-lock` now uses only `ckb_std::syscalls` too.
- **Path selection / error codes**: accepted = refund conditions hold, or the run rules hold. If the refund
  conditions fail and the transaction has **no registry cell dep**, the lock reports 14 ("neither path"); with
  a registry dep it reports the run-path error. So "refund before expiry" and "refund without refund-lock
  input" are both error 14; error 10 now means "several registry deps / malformed registry data".
- **One job input per transaction** = no other input whose lock has the job lock's code hash (any args, any
  hash type), not only "one cell in the group".
- **Net sums** for both the tip (float lock hash F) and the final refund (`refund_lock_hash`): Σ outputs − Σ
  inputs with that lock hash. The concept states the refund as Σ outputs; net is stricter and protects
  refund locks that do not need a signature (anyone-can-pay).
- `remaining_runs > 1` → recreation required (exactly one output with byte-identical job lock and type); `≤ 1`
  (0 only reachable without the job type) → consumed with refund. Outputs with the same lock/type are not
  forbidden in the last run (a new job may be created in the same tx).
- Type update additionally requires the output's `remaining_runs ≥ 1` (an Every job with 1 run left must be
  consumed, never recreated with 0).
- Epoch triggers are compared as exact fractions; `length = 0` is read as a whole epoch.
- The job lock does not check registry attestation expiry or bind runs to a runner key (documented as limits
  in `spec/job-cell-standard.md` §3).
- The harness test now creates a real job (type script) and refunds it after expiry (job lock).

## A3a-fix Phase 1 – float lock and registry (audit F1, F3, F4)
- **F4: the float never pays fees.** `csr-float-lock` runner path: float output capacity ≥ Σ float inputs of the
  group (no loss at all, error 8). Job fees already come from the job cell (A3a Phase 4), so nothing needs the
  old allowance. `max_fee_shannons` stays at `args[64..72]` (72-byte layout unchanged: existing float lock
  hashes, CSR config and tests keep their format) but is **reserved/unused**. Owner path unchanged.
- **F3:** runner path requires the single float output to have no type script and empty data (new error 10,
  `ERR_OUTPUT_SHAPE`). Checked after the output count (7) and before capacity (8).
- **F1:** `csr-registry` rule 6 / `ERR_NO_RUNNER_KEY = 17`: on creation, and on the owner path when
  `service_pubkey` **or** the dual lock's runner hash changes (stricter than "pubkey changes": an owner cannot
  switch to another sighash form of the same key without that key either), an input with the output's runner
  lock hash must be present. Checked after rules 1–5, so the existing tests keep their error codes (e.g. a
  mismatched runner lock still reports 13). The runner path needs no extra check: the dual lock already
  requires owner or runner key, and the runner path keeps the lock identical.
- Tests: `float_runner_fee_within_max_ok` / `float_runner_fee_above_max_rejected` deleted (replaced by
  `float_runner_capacity_loss_rejected`, `float_runner_no_job_drain_rejected`, plus `float_runner_keeps_capacity_ok`);
  `float_runner_merge_two_floats_ok` now merges without loss. New helper `Env::registry_create_tx` (inputs
  wallet + runner key cell, both signed). `registry_create_cycles_of_type_script` measures the type script as
  creation tx minus a baseline that only spends the key cell (the runner key's system lock now always runs).
  `registry_create_with_genesis_secp_runner_lock_ok` deploys the sighash binary under the genesis Type ID type
  script (args `0x8536c9d5…c304`, hash asserted = `0x9bd7e06f…`) so that the runner key cell can be spent.
  Extra test `registry_owner_changes_service_pubkey_with_old_runner_key_rejected`.
- **Downstream impact:** `csr register prepare` (A3b) must add the node's runner key cell as input and output
  and sign that input with the service key (also for an owner-path key rotation: the **new** key cell). Runner
  transactions that spend the float must pay the miner fee from another input (runner key cell or job cell).
  Job transactions (A3c) must add the monitor config cell as a cell dep (Phase 2, F5).
- **F8 (documented, `spec/registry.md` §4):** one key holder may still create several registry cells;
  consumers pick the one with the latest attestation.

## A3a-fix Phase 2 – job lock (audit F2, F5) and documentation (F6–F8)
- **F2: runs are bound to the registered node.** `csr-job-lock` run path rule 3: the registry cell dep's lock
  must parse as a script with 64-byte args (else 10, `ERR_REGISTRY_DEP`); an input with lock hash
  `args[32..64]` (the node's runner key cell) must exist, else `ERR_NOT_REGISTERED_RUNNER = 15`. The dep lock's
  code hash is not checked: only cells created through `csr-registry` carry its type script, and that type
  script already forces a `csr-dual-lock`.
- **F5: expired attestations stop earning.** Rule 4: `config_type_hash` = the registry dep's type script args
  (32 bytes, else 10); exactly one cell dep with that type hash (`ERR_CONFIG_DEP = 16`, also used for config
  data shorter than `monitor_pubkey ‖ clock_ms` = 41 bytes – no extra code, the concept fixes the code list);
  `attestation.expiry_ms > clock_ms` (`ERR_ATTESTATION_EXPIRED = 17`, same strictness as registry rule 4). The
  signature is not re-verified (it was verified when the cell was written; saves ~7.9 M cycles per run).
- **Rule order**: trigger (8/9) → tip (11) → registered runner (15) → live attestation (16/17) → recreation /
  refund (12/13). The new checks come after the tip so that the existing `job_tip_to_unregistered_float_rejected`
  (impostor's own float and key) keeps reporting 11; with the tip to the registered float the impostor gets 15.
- Tests: `S::run_tx` adds the monitor config cell as a second cell dep (the registered runner key cell was
  already an input). New helpers `S::replace_dep`, `S::config_at(clock_ms)` (same Type ID type script, new
  clock = a later version of the config cell) and `S::registry_with_runner(hash)` (registry data and type with
  another dual lock, created directly in the test context – the "forged registry" a type script would never
  have accepted on creation, which is also how a dep from another deployment would look). Extra tests
  `job_run_with_malformed_registry_lock_rejected` (10) and `job_run_with_two_config_deps_rejected` (16).
  `job_lock_cycles_of_run` now uses a registry dep whose runner is the always-success wallet, so it still
  measures the job lock without a system lock: 51,406 cycles (was 38,219); whole OnDemand run 1,779,894.
- **F6/F7/F8 documented** in `spec/job-cell-standard.md` §3 "Limits" (F8 also in `spec/registry.md` §4):
  up to `max_fee_shannons` of the job capacity may go to any output (bounded by the consumer's choice); a cell
  with only the job lock is the creator's risk, CSR discovers jobs by type and lock; several registry cells per
  key are possible, consumers pick the latest attestation.

## Downstream impact (A3a security fixes F1–F5)
- **A3b – `csr register prepare`** must add the node's runner key cell (system lock of the service key) as an
  input and recreate it as an output, and sign that input with the service key – on creation and on an
  owner-path key rotation (then the **new** key's cell) (F1).
- **A3c – job transactions** must add the monitor config cell (Type ID, `config_type_hash` = registry type
  args) as a cell dep next to the registry cell, include the runner key cell as an input signed with the
  service key (F2, F5), and must not run with an attestation that has expired by the config cell's `clock_ms`
  (refresh first).
- **The float never pays fees.** `max_fee_shannons` in the float lock args is reserved/unused (layout of 72
  bytes unchanged); a runner transaction that spends the float recreates it without type script or data and
  with at least its input capacity, and pays the miner fee from the job cell or the runner key cell (F3, F4).
- Off-chain consumers (CSR, monitor, CRT) that see several registry cells for one key pick the one with the
  latest attestation (F8); CSR ignores cells that carry the job lock without the job type (F7).
# Decisions (A3a audit fixes)

The project's decision log is `docs/decisions.md` (sections "A3a-fix Phase …"). Short list for this run:

- Phase 1: F1/F3/F4 implemented as specified; new error codes float 10 (`ERR_OUTPUT_SHAPE`), registry 17
  (`ERR_NO_RUNNER_KEY`). Rule 6 of the registry also triggers when the owner changes only the runner lock hash
  (not only `service_pubkey`), and is checked after rules 1–5 so existing error codes are unchanged.
- Obsolete float fee tests deleted; see `docs/decisions.md` for the test-helper changes.
- Phase 2: F2/F5 in `csr-job-lock` (errors 15, 16, 17), checked after the tip (11) so existing codes stay.
  Short config data also reports 16. The dep lock's code hash is not checked (only args length, as specified).
  Extra tests: malformed registry lock (10), two config deps (16). Docs: job/registry specs, decisions
  ("Downstream impact"), milestone "Security fixes", CHANGELOG (also an entry for Phase 1).

## A3b Phase 1 – devnet harness
- Genesis/funding account = dev-chain key `d00c06…` (20 B CKB, args `0xc8328a…`); block assembler = second well-known key `63d867…` (args `0x470dcd…`) so mining rewards don't mix with the funding account. Monitor key is an invented fixed test key. All in `devtools/devnet-keys.json`.
- Pinned ckb v0.210.0 `x86_64-unknown-linux-gnu` (non-portable build; works here), SHA-256 `68a94c19…7bdf`.
- Indexer RPC module enabled; `Subscription`/`Terminal` dropped from the default module list, `IntegrationTest` added.

## A3b Phase 2 – `csr-dev` and the devnet network
- `csr-dev` (`devtools/csr-dev`) uses `ckb-types`/`ckb-jsonrpc-types`/`ckb-hash` + blocking `reqwest` (rustls) + `secp256k1`; no `ckb-sdk`, so no openssl/aws-lc in `Cargo.lock`. Keys from `devtools/devnet-keys.json` are embedded at compile time. On testnet the single key in `CSR_DEV_TESTNET_KEY` serves as funder and monitor.
- Deployment: one transaction per script (each waits for commit); code cells are locked by the funder. The config cell holds 10,000 CKB; `clock-update` pays its fee from that cell's capacity, and its data is `pubkey ‖ clock_ms ‖ 8 reserved zero bytes`.
- The deployments file has one extra key `config_cell.type_args` (Type ID args) so tools can find the config cell via the indexer; `clock-update` moves the cell and rewrites `config_cell.tx_hash/index` in the file. `attest`/`clock-update` take `--deployments` (default `deployments/devnet.json`).
- Attestation file: `{"peer_id","service_pubkey","network","network_id","expiry_ms","signature","monitor_pubkey"}`; `peer_id` bytes = UTF-8 of the node ID string; default expiry = on-chain `clock_ms` + 7 days.
- Fees of `csr-dev` transactions: 2 shannons/byte + 1000. `sign-tx` fails (exit 1) if the key signs no input; it fills the witnesses and the `signatures` map of the ckb-cli tx file.
- `csr`: `network = "devnet"` (also `--network devnet`) only under `cfg(debug_assertions)`; `[devnet]` is rejected for other networks; `[devnet]` is optional for `network = "devnet"` (chain commands of later phases will require it). Expected chain `ckb_dev`. R2 skips the "no outbound peers" rule when the expected chain is `ckb_dev`. Devnet reuses the testnet network defaults file (monitor URL/pubkey) as placeholder.
- `csr identity --json` creates the keys if missing; runner lock = `secp256k1_blake160_sighash_all` (hash type `type`) with `blake160(service pubkey)`.

## A3b Phase 3 – runner key cell, float, operator signing flow
- `runner/src/chain.rs` builds the transactions with `ckb-types` (no `ckb-sdk`); same fee rule as `csr-dev` (2 shannons/byte + 1000). The unsigned file already carries a 65-byte zero lock placeholder in the witness of the first owner input, so the fee is exact after signing.
- Float and registry cells are found via the node's default indexer, which has no `partial` search: the float is found by scanning all cells of the float code hash (prefix `0x`, paginated) and matching the runner lock hash at args offset 32.
- Owner-path transactions (create/top-up/withdraw) pay the fee from the owner's plain cells, so the float changes by exactly the requested amount. Top-up/withdraw learn the owner's lock from the inputs of the transaction that created the float cell; if that fails (e.g. after a runner-path spend) `--owner-lock-args` is needed (optional flag). Only `secp256k1_blake160` owners are supported.
- `prepare-withdraw` has `--all` (closes the float) as the alternative to `--ckb N`; otherwise the float must keep its occupied capacity (113 CKB). `prepare-create` refuses if a float already exists. Default max fee 1,000,000 shannons (reserved/unused by the lock).
- `runner-cell status` reports the first cell's outpoint and the total capacity of all plain cells under the runner lock. `tx submit` only broadcasts (no waiting); errors carry the node's message.
- HTTP: `GET /api/runner-cell`, `GET /api/float`, `POST /api/float/prepare-{create,topup,withdraw}` (JSON body `ckb`, `owner_lock_args`, `all`; returns `{file, fee_shannons, tx_hash}`), `POST /api/tx/submit-signed` (body = the signed file). Dashboard page "Float" with Create / Top up / Withdraw buttons and a signed-file upload. Register button and `/api/registry/prepare-create` come with the register flow.
- testnet/mainnet: chain commands fail with "scripts not deployed" until shipped hashes exist.

## A3b Phase 4 – registration
- Registry creation tx: inputs `[runner key cell, owner cells…]`, outputs `[registry, identical runner key cell, owner change]`; cell deps secp256k1, registry code, config cell (found via indexer by its Type ID script, not by the deployments file's outpoint, which `clock-update` moves). Registry cell capacity = exactly its occupied capacity. Only the first runner key cell is used.
- Input 0 (runner cell) is signed by CSR (sighash group of one input); the owner placeholder witness is at index 1.
- Extra refusals beyond the spec: expired attestation (vs on-chain `clock_ms`), unknown network name. The attestation `peer_id` is not compared with the node's (the node's peer ID is only known to a running service).
- `register status` picks, among registry cells with this node's service pubkey, the one with the latest attestation expiry (F8).
- The registry owner is not required to equal the float's owner.

## A3c Phase 1 – job creation, discovery, selection
- `csr-dev job`: capacity = occupied capacity + `runs × (tip + max_fee)` + 1 CKB; job type/lock referenced by data2 hash (from the deployments file, which also gives `registry_type_hash`); `runs > 1` only for `every:N`.
- Discovery searches the indexer by the exact job lock (`csr-job-lock`, args = this deployment's `registry_type_hash`) and keeps cells whose type is `csr-job-type` with 32-byte args; malformed job data is skipped. Found jobs are stored in SQLite table `job_cells` (`discovered` / `excluded` + reason), refreshed on every new block by a loop in `csr run` that only works while the runtime state is `on`.
- `trigger_not_due` is evaluated at the current tip: `at-block:n` → tip ≥ n; `every:k` → tip ≥ creation block + k; epoch via cross-multiplied fractions; timestamp via the tip's median time (whole seconds). Exclusion order is exactly as specified; `csr jobs plan` ignores `runtime.enabled` (diagnostic only).
- Allowlist = DB status `approved_enabled` AND `runtime.allowed_services`. `service add-onchain` stores language `onchain`, name = first 8 hex chars of the data hash.
- Heartbeat is a copy of `kvtest` semantics (wrapping u32 counter in `counter`, result = new value), manifest `heartbeat.toml`.

## A3c Phase 2 – execution, settlement, Jobs page
- New module `runner/src/keeper.rs`, driven by the discovery loop of `csr run` once per new block. Runtime `off`/mainnet-gated → only transactions already submitted are tracked; R2 not synced or a "node always wins" hold (`resources::hold_reason`) → the next batch is set to `Paused` (resumes to `Running` at the next block without hold). A VM is not interrupted mid-run (runs are short and wall-time capped).
- One run row per job cell outpoint (a recreated `Every` cell is a new outpoint → a new run). A selected job gets a run `Discovered → Queued`; a batch of at most `max_concurrent_jobs` (best tip rate first) goes `Scheduled → Running → Completed → Submitting`. A new batch starts only when no run is `Running/Completed/Submitting`.
- Runs of a batch execute in parallel per service (runs of one service sequentially, they share its storage). Their transactions are submitted one after another, each **chained** on the previous one's float and runner key cell outputs (tx-pool chaining), so two jobs can settle in the same block.
- Run transaction: inputs `[runner key cell, job (since), float, skeleton inputs…]`, outputs `[runner key cell identical, float + tip, recreated job (data with runs − 1, capacity in − tip − fee) | refund (lock = refund lock, capacity in − tip − fee), skeleton outputs…]`, cell deps `[secp256k1 dep group, job lock, job type, float lock code cells, registry cell, monitor config cell, skeleton deps…]`. The runner key cell is input 0 and signed with the service key. Fee = 1,100 shannons/KB of the serialized tx (+4 bytes), from the job; above `max_fee_shannons` → `Failed`. The service code cell is not a cell dep (the service is not a script of the transaction).
- `since`: OnDemand 0, AtBlock n, AtEpoch `0x20…` (length 0 normalised to `n/1`), AtTimestamp `0x40…` with `ceil(ms/1000)` s, Every `0x80…` relative k.
- The refund lock script is found by walking back from the job cell's creating tx (its outputs and spent inputs, following previous job cells, ≤ 64 hops) and cached in the `settings` table (`refund_lock:<hash>`). If it cannot be found (e.g. `--refund-lock-args` of a lock that never appeared on chain), the **last** run fails; earlier runs are not affected.
- Transaction skeleton = molecule `Transaction`; appended after CSR's own cells; its outputs must be covered by its own inputs. Documented in `spec/host-interface.md`.
- `chain_read` (7003: kinds 0 cell output, 1 cell data, 2 header, 3 transaction) and `cells_query` (7004: script prefix, lock/type, ≤ 64 out points as `OutPointVec`) — ABI in `spec/host-interface.md`. Node calls from the VM thread use `Handle::block_on` on the main runtime. No test service exercises them yet (heartbeat does not need them).
- Service code: `service add-onchain` now stores the code cell outpoint (`services.outpoint`, added by migration); the code is read from the live cell, checked against the code hash and cached in memory.
- Tracking: `committed` → `Settled` + settlement row (tip = the job's tip, block from `tx_status`); `rejected`, unknown for > 30 blocks or pending for > 500 blocks → `Lost` if the job cell is spent, else `Rejected`. Send errors: same rule. A waiting run whose job cell disappears → `Lost`; whose job expired → `Expired`. Failed/rejected outpoints are retried at most 3 runs in total.
- Restart: `Running`, `Scheduled`, `Paused`, `Completed` → `Queued` (VM results are not kept; the run executes again).
- `GET /api/jobs` lists runs newest first; `GET /api/jobs/{id}` adds `timeline` (`t`, `state`, `note`), `logs` (the VM debug lines, ≤ 64 × 1 KB), `result_hex`, `error`, `limits`. `GET /api/earnings` settlements also carry `t`; `float.capacity_shannons` is read from the chain (null if unavailable); `fees_shannons` is always 0.
- Jobs page tabs: Pending (Discovered/Queued/Scheduled), Active (Running/Paused/Completed/Submitting), Closed (Settled), Failed (Failed/Rejected/Expired/Lost); a row opens the detail (timeline, logs, limits).

## A3c Phase 3 – race, Earnings page
- A send error naming a dead/unknown input, double spend, conflict or RBF means another transaction took the job cell; the run ends in `Lost` even though the cell is still live on chain (the winner is only pending). Other send errors keep the live-cell check.
- Earnings page: balance and totals from `/api/earnings`, tips-per-day bar chart (UTC days of the settlement time), settlements table; Top up / Withdraw are links to the Float page, which owns the file flow.

## A4 Phase 1 – release
- Release profile without LTO (`codegen-units = 16`); size checked against the 30 MB target.
- Runtime toggle is a disabled checkbox in the Overview runtime card (no toggle existed before).
- `LICENSE` is packaged only if present (none in the repo).

## A5 – Testnet registration

- **Embedded testnet deployment.** `deployments/testnet.json` is compiled into the runner (`include_str!`), so a testnet node needs no deployment file. `load_deployments` chooses by network (testnet: embedded, devnet: `[devnet] deployments` file, otherwise "not deployed yet"); `parse_deployments(text, source)` holds the JSON parsing.
- **Testnet monitor defaults.** `config/networks/testnet.toml` now carries the real monitor `https://monitor-testnet.vericell.net` and its key (also the owner of the config cell). `mainnet.toml` keeps its placeholders.
- **`csr register attest`.** `runner/src/attest.rs`; needs only `local_node_info` (no deployments, no sync). Monitor URL/key come from `[monitor]` when non-empty, else the network defaults, like the outbound check-in. The node is asked first, so an unreachable node fails before the monitor is contacted.
- **Verification before writing.** The attestation is checked (peer, service key, network, expiry, recovered signer) and only then written, exactly as the monitor returned it (pretty-printed). `attestation_message` is duplicated from `csr-dev` (the runner must not depend on devtools).
- **Error reporting.** `403 not_eligible` gets a hint (checks in the last 24 hours, `[ranking] join = true`); other non-200 replies report `monitor replied <status> <error>`.
- **Output.** The printing of `chain_cmd` moved to the shared `print_result`.

## A6: CSR monitor map
- **Shared `worldmap.js`.** `runner/assets/worldmap.js` is a byte-for-byte copy of the file the monitor's public page uses (Natural Earth 1:110m shapes, public domain). It is never edited here, so both pages draw the same map.
- **No tile server or CDN.** The map is inline SVG from a local script: no external requests, the CSP stays `'self'`, the dashboard works offline and leaks nothing, and all assets stay under the 200 KB budget.
- **2° cells.** The monitor publishes only centres of 2° × 2° cells, so exact positions are never exposed; the credit line says dots are areas, not places. Coordinates are display-only, never logged or stored.
- **Unknown locations are not drawn.** Runners without a location are only counted (`unlocated`). A missing or failing `/v1/locations` yields a map without dots and the note "Location data is not available from the monitor."; it never marks the network data stale.

## A7 – Stake as a referenced DAO deposit

### D-A7-1: the registration stake is a referenced Nervos DAO deposit; the float cell is removed
- **Stake = a referenced DAO deposit.** The registry cell names a normal Nervos DAO deposit cell of the operator in the new last field `stake` (`OutPoint`). `csr-registry` checks it on creation and on owner-path updates that change `stake` or the owner (`spec/registry.md` §4.1, errors 18–21): a `code` cell dep equal to `stake`, DAO type script (`0x82d76d1b…3f2e`, `type`, empty args), data = 8 zero bytes (deposit, not withdrawing), capacity ≥ `MIN_STAKE`, lock hash = the registry's `owner_lock_hash`. The deposit is never spent by CSR and keeps earning DAO interest; making it is left to the operator's wallet. CSR never holds or moves operator funds; it only prepares unsigned transactions.
- **`MIN_STAKE` = 10,000 CKB, hard-coded** in the contract (`1_000_000_000_000` shannons), the same on every network. A config value would need a trusted config cell; a constant is auditable and cannot be changed under a live registration.
- **Where the stake sits in the cell deps, and why.** A script can read a dep cell's fields but not its out point. The contract therefore finds `stake` in the raw transaction's `cell_deps` (`load_transaction`) and maps its position *j* to `Source::CellDep` index *j*. That mapping only holds while no `dep_group` entry precedes it (a dep group expands to its members), so the stake entry and every entry before it must be `code`. Builders put the stake **first**; the secp256k1 dep group comes after it.
- **Left to the CSR Network Monitor** (a contract cannot see later spends or other registrations): the deposit is *still deposited*, and *one deposit per registration* (several registry cells naming the same deposit → the oldest wins). The monitor stops attesting registrations that fail these checks; their attestations then expire and their job runs fail (job lock error 17).
- **Float removed.** `csr-float-lock`, the float cell and all `csr float …` commands are removed. The float was a CSR-specific lock that received tips and that the runner could add to; a standard ACP cell does the same with an existing, widely deployed script that wallets understand, so one CSR contract less has to be audited and deployed.
- **Payout = an ACP cell.** Tips go to the operator's payout cell, a standard anyone-can-pay cell (20-byte args, no minimum, no type, empty data). The registry stores its lock hash as `payout_lock_hash` in the old `float_lock_hash` slot; `csr-job-lock` keeps the rule "net inflow to `payout_lock_hash` ≥ tip" (error 11). The runner can top the cell up without the owner's signature; only the owner can withdraw.
- **No slashing, no job collateral in CSR.** The stake is a sybil cost and a visible commitment, not a bond. Applications that need a bond define it in their own contracts.
- **Open points.** (1) A per-network `MIN_STAKE` (e.g. lower on testnet, re-evaluated before mainnet) would need a new contract build per network; decide before the mainnet deployment. (2) One deposit per *runner* (not only per registration) across several registrations of the same operator is not enforced on chain; the monitor's "oldest wins" rule covers the per-registration case only.

### A7 Phase 1 – contracts
- **Registry v2 only.** The 7-field table is required; v1 data is error 7 in `csr-registry` and error 10 as a job-lock registry dep. No migration path in the contract: the testnet deployment is redeployed (manual phase), and an owner update from an unparsable input counts as a stake change, so the stake rules apply.
- **Stake rule 1 scan.** The scan stops at the first non-code entry; a match after it (or `stake` itself as `dep_group`) is error 18, as is a malformed raw `cell_deps`. The raw transaction is read in pieces (header, raw header, `cell_deps` only), so large witnesses do not have to fit into memory.
- **Owner change** is "output `owner_lock_hash` ≠ input `owner_lock_hash`" (dual-lock `args[0..32]`). A transfer must therefore come with a deposit of the **new** owner.
- **Test harness.** The DAO type script does not run for a cell dep, so no DAO binary is deployed. `Env::stake` creates a 10,000 CKB deposit; `Env::registry_create_tx` puts the stake of the data (its last 36 bytes) first in the cell deps and creates a deposit owned by the wallet there if none exists. Job tests use an always-success payout lock (the job lock only compares lock hashes).
- **Comments in the remaining contract `Cargo.toml`s** pointed at `csr-float-lock` for the `bytes`-crate atomics note; they now state it directly.
- **Runner and `csr-dev` unchanged in this phase.** They still build and their tests pass, but `csr-dev deploy-contracts` still lists `csr-float-lock` and the runner still builds v1 registry data; both are switched to v2/ACP in the following phases. Until then the devnet flow (used only by the obsolete float acceptance tests) does not work.

### A7 Phase 2 – deployments, csr-dev and the payout cell
- **Vendored ACP.** `devtools/vendor/anyone_can_pay` is the binary of the `ckb-sdk` 3.7.0 crate (`src/test-data/anyone_can_pay`), fetched with `cargo fetch` in a throw-away manifest outside the repo; ckb-sdk is not a dependency of anything. `devtools/vendor/SHA256SUMS` pins it and `csr-dev deploy-contracts` refuses a binary with another SHA-256 (via `sha2`, already used by the runner). It is deployed as a code cell and referenced with `hash_type data1` (data hash `0xcd69ba81…31e6`).
- **Deployments format.** `scripts.anyone_can_pay` carries `dep_type` (`code` | `dep_group`); every other entry may omit it (= `code`), an unknown value is an error. The runner builds all cell deps of a deployed script from its `dep_type`. On testnet `csr-dev` writes the public ACP deployment (dep group) instead of deploying the binary.
- **`--keep-config`** copies `config_cell` and `monitor_pubkey` verbatim and derives `registry_type_hash` from its `config_cell.type_hash`; the file is read before anything is deployed.
- **`dao-deposit`** finds the DAO code cell in the genesis block by its type script hash (`0x82d7…2f2e`), so it works on the devnet and on testnet without per-chain constants.
- **`sign-tx` signs lock groups.** Inputs are grouped by their full lock script; a group is signed if it is the key's secp256k1 lock or an ACP lock (code hash of the public testnet deployment or of the `--deployments` file, default `deployments/devnet.json`) with exactly the key's 20-byte args. The ACP signature path uses the secp256k1 sighash-all message, so the same signing routine serves both. `signatures[<args>]` lists one signature per group.
- **`payout.lock_args`** is `""` or `0x` + 40 hex digits; anything else fails config validation (and so startup and `PUT /api/settings`). `csr payout …` checks it before contacting the node, so the error does not depend on node availability.
- **Finding the payout cell.** Exact indexer search for the ACP lock with `payout.lock_args`, no type, empty data; if several exist (anyone can create one), the oldest (first in ascending order) is the payout cell.
- **Withdrawing small amounts.** A withdrawal sends the amount minus the fee to the secp256k1 lock of `payout.lock_args`. If that is below the 61 CKB a cell needs (e.g. withdrawing 30 CKB), one plain cell of that lock is added as an input and the amount is added to it; the transaction then has two lock groups (ACP and secp256k1), both signed by the same wallet key. Without such a cell the command fails and asks for a larger amount.
- **Registration and the payout cell.** `register prepare` needs `payout.lock_args` (it writes the payout lock hash) but not the cell itself: anyone can create it later. The keeper's run context needs the cell and refuses to run if the registry names another payout lock hash. Run transactions spend the payout cell on the ACP payment path (empty witness, capacity grows by the tip) with the ACP cell dep (dep group on testnet); the secp256k1 dep group already in every run supplies the secp256k1 data ACP needs.
- **Registry data in this phase** is still built with 6 fields (phase 3 adds `stake`); the parser already accepts 6 and 7 fields and is unit-tested against the monitor cross-check vector.
- **API and earnings.** `/api/float…` is replaced by `GET /api/payout`, `POST /api/payout/prepare-create`, `POST /api/payout/prepare-withdraw` (same JSON as the CLI). `GET /api/earnings` reports the balance as `payout.capacity_shannons` (was `float`); `register status` reports `payout_lock_hash`. The dashboard only got a null-safe read of the earnings balance; the float page is reworked in phase 5.

### A7 Phase 3 – registration with the DAO stake
- **Out point syntax.** `--stake` (and `stake` in the API) is `0x` + exactly 64 hex digits, `:`, a decimal `u32` index; hex in any case is accepted and normalised to lower case (what is sent to the monitor and shown by `register status`). The `0x` prefix is required. `register attest` parses it before reading the config or asking the node, so a malformed value never reaches the monitor.
- **Check order of `register prepare`:** synced node → attestation (key, network, expiry by the on-chain clock) → payout cell exists → runner key cell → owner lock args → stake (syntax, live, DAO deposit, ≥ 10,000 CKB, lock hash = secp256k1 lock of `--owner-lock-args`). The runner checks the same rules 2–4 as the contract (`stake_problem` in `runner/src/chain.rs`) so the operator gets a clear message instead of a script error. The payout cell is now required (phase 2 only needed `payout.lock_args`): a registration without a payout cell could not be paid tips; `payout_lock_hash` is the hash of that cell's lock.
- **Cell dep order.** `assemble` takes the stake dep separately and puts it at index 0, before the secp256k1 dep group; other prepared transactions are unchanged (secp256k1 first).
- **`register status` stake.** Rules 2–4 are evaluated now against the registry cell's `owner_lock_hash` (dual-lock `args[0..32]`). `withdrawn` = the cell is not live (spent – e.g. a DAO withdrawal phase 1 – or unknown), `capacity_shannons` is then `null`. A 6-field (v1) registry cell has no stake: `stake` is `null` even though `registered` is true.
- **Monitor errors.** A non-200 reply prints `monitor replied <status> <error>[: <reason>]`, plus a hint for `stake_invalid`/`stake_in_use`. The reason codes and statuses in `spec/monitor-public-api.md` (`not_found`, `not_dao_deposit`, `too_small`, `wrong_owner`; `stake_in_use` = 409) are CSR's proposal for the monitor project; CSR displays whatever `reason` it gets and does not depend on the status number.
- **Dashboard.** The registration row got a stake input and the status list a stake line; the page is otherwise reworked in phase 5.
- **Phase 4: keeper pays into the payout cell.** The runner-side pieces (`run_context`/`build_run` with inputs `[runner key cell, job, payout cell, …]`, empty ACP witness, secp256k1 + ACP deps, refusal when `payout_lock_hash` differs from the configured payout lock, chaining on the payout output, `GET /api/earnings` with `"payout": {"capacity_shannons"}`) landed together with the payout cell in phase 2; phase 4 verified them with `acceptance/a7/p4.test.mjs` and needed no further code changes. Without `payout.lock_args`, payout cell, runner key cell or registration `run_context` errors, the keeper reports the reason and runs nothing.

### A7 Phase 5 – dashboard, monitor stake, documentation
- **One page for payout and stake.** `#/float` is replaced by `#/payout` ("Stake & payout"): payout cell status and Create/Withdraw (empty amount = withdraw everything), a registration block (status incl. stake out point, status and CKB; Register with attestation file, owner lock args, stake), and one shared "submit signed file" card using the existing `POST /api/tx/submit-signed`. The owner lock args input is shared by Create and Register.
- **Overview / Earnings.** The Overview card is "Payout" (balance, tips, settlements, link "Stake & payout"); the Earnings page shows `earnings.payout` and links to `#/payout` instead of Top up (an ACP cell is topped up by a plain transfer from any wallet). Settings gets `payout.lock_args`.
- **Monitor stake pass-through.** `NodeStats.stake` is `Option<serde_json::Value>` with `#[serde(default)]`; `/api/network` serialises it as `node.stake` (`null` for an older monitor). The runner does not interpret it; the "My rank" card shows `status (N CKB)` when present.
- **Docs.** `spec/float-lock.md` became `spec/dual-lock.md` (authorisation by key cell + dual lock; float dropped). `docs/spec/CSR-SPEC-v0.2.md` is the historical v0.2 text and was left unchanged. The `stake` object and its statuses (`deposited`, `withdrawn`, `invalid`, `duplicate`, `none`) in `spec/monitor-public-api.md` are CSR's proposal for the monitor project, like the attest reasons.

## A8 – The runner wallet

### D-A8-1: the runner wallet; the stake owner consents by a signed message
- **Why.** Wallets (Neuron and others) can *sign a message* but cannot sign CSR's transaction files, so A7's wallet-signed flows (registration prepared by CSR, signed in a wallet, submitted back) were the hardest step of a setup. With A8, setting up a node needs one normal transfer (about 500 CKB to the runner address) and one signed line of text; CSR signs every transaction itself with the service key.
- **Runner wallet.** All plain cells of the runner's own secp256k1 lock (`blake160(service_pubkey)`, A7's "runner key cell") form the runner wallet; CSR shows its address.
- **The registry is owned by the runner wallet:** dual-lock args = `runner_lock_hash ‖ runner_lock_hash`, paid from the runner wallet; removing it returns the capacity there. `csr-registry` allows any registry owner; when owner = runner, every update the runner signs takes the owner path.
- **Stake consent replaces "stake owner = registry owner"** (stake rule 4, `spec/registry.md` §4.1–4.2). The DAO deposit (≥ 10,000 CKB) stays in the operator's wallet. Its owner signs `CSR-STAKE-CONSENT-v1|<network>|0x<stake tx hash>:<index>|0x<service_pubkey>|<peer_id>` ("Sign message", digest `blake2b-256("Nervos Message:" ‖ text)`); the signature travels in `WitnessArgs.output_type` of the registry output's witness and is checked on creation and whenever the stake or the owner changes (errors 21/22/23). Each field is there to stop a replay:
  - **network** – a testnet consent cannot be used on mainnet (or devnet) for the same deposit/key;
  - **stake** (out point) – the consent covers exactly one deposit; signing for one deposit does not lend another, and a new deposit needs a new consent;
  - **service key** – the deposit backs one runner; a copied consent cannot back another node's service key (with rule 6 only the key holder can register it anyway);
  - **peer id** – binds the consent to the node identity the monitor attests, so the same signature cannot be reused for another node under the same key.
  A fixed prefix `CSR-STAKE-CONSENT-v1` makes the text recognisable to the signer and versioned. Payout and attestation are not in the text: the attestation is renewed constantly and the payout is the runner wallet itself.
- **Tips go into the runner wallet:** `payout_lock_hash = runner_lock_hash`. The job lock is **unchanged**: the settlement already spends a runner wallet cell (F2), whose output now holds its capacity plus the tip; the fee comes from the job cell, so the wallet grows by exactly the tips and pays its own small fees (registry refreshes, sweeps). The anyone-can-pay payout cell is no longer needed.
- **Risk: earnings sit on a hot key** (the service key lives on the node) until swept. That is why the runner sweeps automatically: at `sweep_threshold_ckb` (default 1,500) it sends `sweep_amount_ckb` (default 1,000) to the operator's `sweep_address`. The stake itself never leaves the operator's wallet, and a stolen service key cannot move it; the monitor keeps checking the deposit is still in the DAO (A7, unchanged) and disqualifies the registration otherwise – the operator then registers again with a new deposit and a new signed text, updating the same registry cell.
- **The stake-owner-input path stays.** An input with the stake owner's lock hash still counts as consent (checked first, no signature recovery), so other tools that build wallet-signed transactions keep working.

### A8 Phase 1 – contract
- **Shared `consent.rs`.** Text building, digest and signature parsing live in `contracts/csr-registry/src/consent.rs` (`no_std`, no allocation: the text is streamed into the hasher). The contract tests include the same file via `#[path]` (and gained a `blake2b-ref` dependency for it), so the vectors test the contract's own code.
- **Error split.** 21 = no consent at all (no stake-owner input; witness missing, not `WitnessArgs`, or no `output_type` – a signature in `lock`/`input_type` does not count); 22 = malformed signature (length ≠ 65, recovery byte not 0–3/27–30, `r`/`s` not a valid scalar); 23 = well-formed signature that is not the stake owner's over this registration's text, **including a signature from which no key can be recovered** (it is parseable, it just proves nothing).
- **High-S** signatures are normalised (`s → n − s`, recovery id low bit flipped) before `k256`, which rejects high-S; the attestation check keeps requiring low-S (the monitor signs low-S).
- **Accepted stake-owner locks** for a signature are the same four `secp256k1_blake160_sighash_all` forms as rule 5 (`type` genesis hash, data hash with `data`/`data1`/`data2`). A deposit under any other lock (multisig, omnilock, …) can only consent by input.
- **`need_stake` unchanged** from A7: creation, or an owner-path update that changes `stake` or the owner.
- **Harness.** `sign_tx` keeps the other fields of an existing `WitnessArgs` (the consent in `output_type` is covered by sighash-all); new helpers `with_witness_type`/`with_consent`, `sign_digest`, `to_high_s`, `recover_pubkey`, `RegistryFields::{consent_text, consent_digest, consent_signature}`, `Env::{runner_wallet_cell, registry_create_runner_owned_tx}`. In the owner/runner-path tests the registry is input 0, so the consent sits in witness 0 next to no lock signature; the runner-owned tests use the concept's layout (runner wallet cell at input 0).
- **A7 tests adjusted.** `registry_create_stake_of_other_owner_rejected` → `…_without_consent_rejected`; `registry_owner_changes_owner_with_old_deposit_rejected` was removed (that transfer is now allowed with the stake owner's consent, see `registry_owner_changes_owner_with_stake_owner_input_ok` / `…_without_stake_consent_rejected`); `registry_owner_changes_owner_with_new_deposit_ok` and `registry_create_cycles_of_type_script` now carry a consent signature, since their stake owner is not an input.
- **Runner, `csr-dev` and the monitor unchanged in this phase.** The runner still builds A7 transactions (owner = operator wallet, which is also the stake owner, so they stay valid); the switch to the runner wallet follows in the next phases, and the monitor's change lives in `csrmon-a8-owner`.
- **Stale runner test fixed.** `chain::tests::embedded_testnet_deployment_parses` still expected the registry code hash from before the A7 testnet redeployment (88d25cd changed `deployments/testnet.json` but not the test); it now expects the deployed `0x4796879e…88eb`. The A8 contract is not deployed yet: `deployments/testnet.json` still names the A7 registry until the redeployment phase.

### A8 Phase 2 – runner and csr-dev
- **The registry cell costs ≈ 540 CKB, more than the "about 500 CKB" of the concept.** *(Superseded by A8b: the docs and the dashboard say about 700 CKB.)* Its occupied capacity is 8 + 97 (dual lock, 64-byte args) + 65 (registry type) + the v2 data (369 bytes with a 46-character peer id, which appears twice: registry and attestation) = 539 CKB; with the ≥ 61 CKB change the wallet needs ≈ 600 CKB for the first registration. No layout the contract accepts gets the cell below ≈ 439 CKB (even with empty peer ids it is 8 + 97 + 65 + 277 = 447 CKB): the dual-lock args, the type args, the attestation and the signature are fixed by the contract and the monitor vector, and the devnet is a real `ckb` node that enforces occupied capacity. The docs therefore say "about 700 CKB", and the runner's refusal names the exact shortfall ("the runner wallet has 500 CKB, but this transaction needs 600.01 CKB … send at least 101 CKB to the runner address …").
- **Devnet faucet in `csr-dev attest` (test accommodation, devnet only).** `acceptance/a8/p3.test.mjs` ("registration paid and owned by the runner wallet …") and `acceptance/a8/p4.test.mjs` ("an Every job runs three times …") fund the wallet with exactly 500 CKB (one `csr-dev fund` cell) and expect the registration to succeed with only runner-wallet inputs; `csr-dev fund` must stay exact (the wallet tests check `capacity_shannons == 250 CKB`), and the tests under `acceptance/` may not change. The attestation is the one devnet step that names the runner (`--service-pubkey`), so on the **devnet only** `csr-dev attest` now also plays the operator's one-time transfer: it sends `--faucet-ckb` (default 200) CKB from the genesis key to the runner wallet (`blake160(service_pubkey)`), logs it on stderr ("devnet faucet: sent 200 CKB to the runner wallet …") and reports it as `faucet {tx_hash, amount_shannons}` in its JSON (`null` when nothing was sent); the attestation file is unchanged. `--faucet-ckb 0` disables it; on the testnet nothing is sent. 500 + 200 = 700 CKB is exactly what the docs ask an operator to send, and a wallet of 150 + 200 CKB is still refused with the "runner wallet" message, so the "too small" test keeps its meaning. This is a dev-tool convenience, not runner behaviour: the real cost of a registration is unchanged and still enforced and reported by `csr register create`. Earlier attempts (five, up to 2026-10-21) had left the two tests failing at the registration step rather than add this side effect; the trade-off was reconsidered because the acceptance suite is the milestone's definition of done and the faucet is confined to the devnet tooling.
- **One wallet transaction builder.** `Chain::wallet_tx` spends runner wallet cells (always inputs `0..k`, as many as needed) plus extra inputs (the registry cell on update/remove), writes `[fixed outputs…, wallet change]`, puts the stake first in the cell deps and the consent into `WitnessArgs.output_type` of witness 0, and signs the wallet lock group with the service key (`sign_wallet_inputs`, which keeps `output_type`, so sighash-all covers it). Fee: 2 shannons per byte + 1,000, paid by the wallet. The change must keep 61 CKB; otherwise the error names the wallet's balance, what is needed and the runner address ("runner wallet").
- **Update vs. create.** `register create` updates the registry cell found for this service key (the one with the latest attestation, F8) only if it is owned by the runner wallet (`runner ‖ runner`); an A7 registration (owned by the operator's wallet) cannot be spent by the runner, so a new runner-owned cell is created instead (`updated: false`) and wins by its later attestation. `register remove` refuses an A7 registration (only its owner can remove it). Update and remove add the dual-lock code dep (the registry cell's lock runs).
- **Consent checks in the runner** mirror the contract: the stake must be a live DAO deposit ≥ 10,000 CKB under the `type` form of `secp256k1_blake160_sighash_all` (the only form a "Sign message" wallet key controls and the one the address shows); the text is rebuilt with the configured network, the stake, this service key and the **attestation's** peer id (`consent-message` uses the local node's peer id, which is what the monitor attests). The runner includes the contract's `consent.rs` via `#[path]` (`runner/src/consent.rs`), so text, digest and signature parsing are the same code; recovery uses the runner's `secp256k1`. Error messages of `create` contain "consent"; `verify-consent` errors contain "signature".
- **`register status` stake** is `deposited` when the cell is a live deposit ≥ 10,000 CKB (rules 2–3); the stake owner no longer has to be the registry owner.
- **Addresses.** `runner/src/address.rs` implements bech32m (encode/decode, no length limit since CKB addresses exceed 90 characters) and accepts only the full format `0x00`; mixed case, other prefixes, a bad checksum, the deprecated short/full (bech32) formats and hex are refused. Devnet uses the testnet prefix `ckt`.
- **Sweep in the daemon** runs in the keeper task (so the runner's transactions go out one at a time): first as soon as the node is reachable, then every 60 s (1 s with `CSR_TEST_FAST_TICK`), independent of `runtime.enabled`; never while keeper runs are `Submitting`; and the keeper starts no batch while a sweep transaction is not committed. A tokio mutex `AppState::wallet_tx` also serialises the keeper tick, the sweep and the API's send/create/remove. `last_sweep` is stored in `<data-root>/<network>/last_sweep.json`.
- **Settlement layout.** The runner keeps its A7 input order: `[runner wallet cell, job, skeleton inputs…]` → `[runner wallet cell + tip, job | refund, skeleton outputs…]` (the job lock does not depend on positions; input 0 being the runner wallet keeps one signing routine). No anyone-can-pay cell or dep any more. `run_context` refuses (keeper runs nothing) when the registration's `payout_lock_hash` is not the runner wallet (an A7 registration): register again.
- **Config.** `[payout] lock_args` is accepted with any value and ignored (`csr run` warns, it is not written back when empty); `sweep_address` must be a full-format address of the configured network; `sweep_amount_ckb ≥ 61`; `sweep_threshold_ckb ≥ sweep_amount_ckb + 100`. The deployments' `anyone_can_pay` entry is no longer read by the runner.
- **Dashboard.** The `#/payout` page became "Wallet & stake" on the new endpoints (wallet status and send; consent text, signature, optional attestation file, register/remove); Overview card "Runner wallet"; Settings fields for the sweep. Further polish follows with the dashboard phase.
- **`csr-dev`.** `sign-message` needs no node unless `--key genesis` (then the funding account of the connected chain, or the devnet genesis key if no node answers); `v` is 0–3. `reclaim` checks every cell first (live, no type, the key's own sighash lock) and sends nothing on a refusal; one output to the key's lock, fee from the cells. `deploy-contracts --reuse-unchanged` (requires `--keep-config`) compares the new binary's data hash with the kept `code_hash` and checks the code cell is live; the output lists `reused`. The anyone-can-pay entry is still deployed as before.

### A8 Phase 3 – dashboard and documentation
- **Page `#/wallet` ("Wallet & registration", `pages.wallet`)** replaces `#/payout`; the nav link and the Earnings button point to it, and the Overview card is `Wallet`. The runner address comes from `/api/runner-cell` (`runner_address`), the balance and sweep data from `/api/wallet`.
- **No browser dialogs:** removal asks "Remove the registration?" with Yes/Cancel buttons inside the page. The text to sign is a read-only textarea with a copy button; the owner address is shown under it.
- **Attestation:** the dashboard no longer offers an attestation file; `POST /api/registry/create` gets `{stake, consent}` and the runner asks the monitor. The button reads "Update stake" when registered.
- **Docs:** *(superseded by A8b)* the "about 500 CKB" text is shown in the dashboard as specified; the README/testing docs keep the more accurate "about 700 CKB" of phase 2 (the registry cell costs ≈ 540 CKB).

## A8b – faucet removed
- **Why the faucet existed.** In A8 phase 2 `csr-dev attest` started sending 200 CKB to the runner wallet on the devnet, because `acceptance/a8/p3`–`p5` funded the wallet with only 500 CKB while a registry cell really costs about 540 CKB (plus change). Tests under `acceptance/` could not be changed then, so the dev tool absorbed the gap.
- **Why the tests were corrected instead.** The runner was right and the tests were wrong: the documented first transfer is about 700 CKB. A dev tool that silently tops up the wallet hides exactly the mistake an operator could make. A8b removes the `--faucet-ckb` option, the transfer, the `faucet` key in the JSON and the stderr line; the attestation file is unchanged. An unknown option now fails as usual.
- **Replacement tests.** `acceptance/a8b/p3`, `p4`, `p5` replace `acceptance/a8/p3`, `p4`, `p5` (same checks, 700 CKB). The A8 originals are obsolete, no longer run, and left untouched.
- **Docs.** README, `docs/testing.md` and `docs/milestones/A8.md` say about 700 CKB to send and about 540 CKB for the registry cell; the devnet flow uses `csr-dev fund … --ckb 700`.

## A8b – dashboard polish
- The runner address is a read-only `div.addr-full` (monospace, wraps anywhere, `user-select:all`) instead of an input with a Copy button, because `navigator.clipboard` is missing over plain http. The Text-to-sign Copy button falls back to `execCommand('copy')` on the textarea and finally says "Press Ctrl+C to copy" with the text left selected.
- Logos sit on a white tile in both themes (simplest, keeps the dark green visible). `ico` is served as `image/x-icon`.

## fix-runner-unranked-node
- `NodeStats.rank`/`of` and the availability values are `Option`, `provisional` defaults to `false`, `last_check_at` defaults to 0; all fields are always serialized (`null` when missing) so the dashboard sees a stable shape.
- An unparsable `/v1/nodes` reply is logged as a warning and becomes `node: None`; the `/v1/network` data is kept. Transport errors and non-404 statuses still fail the refresh.
- Dashboard: `rank == null` shows "not ranked yet" (plus "(provisional)"); `null` availability shows "–"; stake and attestation are shown for unranked nodes. `spec/monitor-public-api.md` documents the unranked shape.
## Skeleton input guard (F-SEC-001)
`Chain::inputs_capacity` now checks every skeleton input with `check_skeleton_input` (pure function over
`SkeletonPolicy`) and fails the run before anything is signed if the input is a runner wallet cell, a dual lock
cell naming the runner lock hash (either half), a job-lock cell, or a registry-typed cell.
## F-RUN-001 – delete_on_consume in csr run
- A background task (own `RpcClient`, checks every 2 s, acts when the tip number changes) asks `get_live_cell` for every rule out point. A cell counts as consumed when the node answers `dead`, or `unknown` while the transaction that created it is committed and has that output (a current CKB node answers `unknown` for spent cells; integration fix, Oct 2026). `live`, cells of unknown or uncommitted transactions, RPC errors and missing methods keep the rules. It runs regardless of the runtime state.
- Deletion log lines go to the tracing log and are appended to `deletions.log` in the storage root, like the CLI does.
## Keeper robustness – phase 2
- **Retry rules unchanged (D14 A):** three attempts per job cell, 30 blocks for an unknown transaction, 500 for a pending one and for a pending sweep.
- **Incomplete discovery (F-FUN-007):** `Chain::job_cells` reads up to 200 pages and returns `JobCells { jobs, complete }`; a cut-off list never marks runs `Lost` (`lost_runs`), and warns once. `get_cells` (20 pages) returns an error instead of a short list.
- **Indexer lag (F-FUN-008):** `indexer_ready` accepts the lag R2 accepts (`LAG_LIMIT`, 5 blocks).
- **Attestation (F-RUN-015):** `run_context` compares the registry `expiry_ms` with the config cell `clock_ms`; an expired attestation is an error, so the batch is held and the runs stay waiting.
- **Sweep (F-FUN-005):** the pending sweep is stored as `"<hash>:<block>"` under `sweep_pending` (cleared with an empty value) and given up after 500 blocks. While it is pending the keeper loop only follows submitted transactions (`Keeper::follow`), it starts no batch.
- **Config bounds (F-FUN-006):** `sweep_amount_ckb` ≤ 1,000,000,000 (was 100,000,000), `sweep_threshold_ckb` ≤ 2,000,000,000, RAM/disk/quota ranges as in the spec; multiplications are checked or saturating.
- **Supervision (F-FUN-004):** all background tasks (R2 guard, sampler, keeper loop, outbound check-in, dashboard, ping responder) are in a `JoinSet`; one ending before a shutdown request ends `csr run` with an error (non-zero exit), and systemd restarts it (`Restart=on-failure`).
- **Pause rule (D4 A):** holds pause only runs that have not started; a started run finishes within `max_job_wall_seconds`. Comments and `docs/architecture.md` corrected; the lifecycle is in `spec/job-cell-standard.md`.
## Dashboard security, phase 1 (D25 B)

- Wallet, registration, service and settings writes need a dashboard password; without one they answer 403 `password_required`. Read-only pages still work without a password on loopback.
- `Host` is checked on every request (421) and `Origin` on writes (403); both are checked before authentication.
- Login: 5 failures per client IP and 50 globally per 15 minutes give 429 with `Retry-After`; Argon2 runs under a 2-permit semaphore.
- Sessions: separate pools (50 authenticated, 200 anonymous); a full pool evicts only its own oldest entry. The cookie gets `Secure` when `x-forwarded-proto: https`.
- Old acceptance tests that wrote without a password (a1/p4, a2/p5, a8/p5, a8b/p5) now set a password and log in, and each has one assertion for 403 `password_required`. a2/p5 and the devnet tests could not be run here (no RISC-V service build / devnet).
- `identity::key_mode_too_open` (fixes the mode to 0600 with a warning) and `wallet::sweep_warning` are in place; their wiring is phase 2.

## Dashboard security, phase 2 (D25 B, D26 A, D28 B)

- **D25 B:** a dashboard password is required only for writes (wallet, registration, service, settings); read-only pages work without one on loopback.
- **D26 A:** the storage key stays on the same disk in `keys/`. A stolen disk or a clear backup is covered by full-disk encryption, as the install guide says; `keys/` is not backed up in clear.
- **D28 B:** no automatic sweep is forced. The runner warns (log once per start, `sweep_warning` in `GET /api/wallet`, Overview and Earnings pages) when the hot key holds at least `payout.sweep_threshold_ckb` and `payout.sweep_address` is empty.
- Files are private: `umask(0o077)` at process start, key files with group/other bits are set to 0600 with a warning, key buffers are zeroized, `LimitCORE=0` turns off core dumps.
## Sandbox storage, phase 1
- Storage format v2 (per-service key and AAD); `migrate_v1` runs once at `csr run` start and logs the count. A file that opens with the v1 rules but whose name does not match its key is skipped.
- `rules.bin` is deleted when its last rule goes; it counts in both quotas, `delete_on_consume` checks the quota on growth. 256 rules per service; a duplicate rule is always `Ok`.
- `rule_outpoints` and `apply_consumed` were missing from the tree, so they are added here (`simulate_consumed` calls `apply_consumed`).
- `limits.max_vm_memory_mb` must be 4; removed from the settings form. `data_cycles` (1 cycle per 4 bytes), `NODE_CALL_CYCLES` (100 000) and `MAX_DEBUG_SCAN` (1024) are defined but not yet charged (phase 2).

## Sandbox storage, phase 2
- Host calls cost 500 + `data_cycles(bytes copied)` (+ `NODE_CALL_CYCLES` for `chain_read`/`cells_query`). The data charge sits in `write_partial` (bytes actually copied out) and `read_mem` (bytes read from VM memory), so every data-moving call is covered; `debug` is charged for the bytes kept. The extra charge uses `add_cycles`, so a job that runs out stops with the cycle-limit error. `debug` scans at most `MAX_DEBUG_SCAN` bytes.
- Example services: see the check below.
- Example services: all of `services/manifests` except the intentionally failing ones (`loop`, `memhog`, `badsyscall`, `bigresult`, `quotatest`) move only a few hundred bytes per call, so the extra charge is a few hundred cycles per job against the 70,000,000 default `max_cycles_per_job`; none needs a higher limit. Nothing was raised.
## csr-runner-fixes
- **Restart flag (F-RUN-009):** `Config::needs_restart` also covers `ranking.join` and `ranking.ping_mode`; both settings are marked "restart required" on the settings page.
- **Disk history (F-RUN-011):** `samples.disk_free_gb` (nullable; added by `ALTER TABLE` to older databases). The Resources page draws only samples that have a value.
- **Service totals (F-RUN-012):** `Db::service_totals` joins `settlements` to `job_runs` on `run_id` and groups by `code_hash` as stored.
- **Monitor key (F-RUN-013):** the mainnet file ships `monitor_url = ""` and `monitor_pubkey = ""`. While the effective key (config, else default) is unusable, the inbound listener answers 503 "no monitor key set", the outbound check-in does nothing, one warning is logged and `/api/network` carries `warning` (shown on the Network page).
- **HTTP clients (F-RUN-014, F-SEC-017):** `network::http_client` (no proxy, no redirects); a 3xx answer is the error "unexpected redirect".
- **Wallet without deployment (F-RUN-017):** `Chain::connect_wallet` fills the deployment fields with unused placeholders; only wallet functions may be used on such a chain. The CLI `csr wallet …`, the wallet routes, `/api/runner-cell`, the earnings balance and the sweeper use it. `mainnet_runtime_unlocked()` stays `false` (decision D5 B). The registry part of the wallet page says "the CSR scripts are not deployed on mainnet yet".
- **Log rotation (F-RUN-018):** `logs::RotatingFile` replaces the start-up rename; `tail` continues with the end of `csr.log.1` when the current file is short.
- **Pending transactions (F-SEC-015):** `wallet::wallet_busy`. This codebase did not store a pending sweep yet, so the sweeper writes its pending transaction hash to the settings key `sweep_pending` (empty = none, cleared at start-up because an earlier process's sweep is no longer followed). The dashboard actions answer 409 `{"error":"busy","detail"}`; there is no registry refresh action.
## Attestation renewal (D3 A)
- The runner renews the registry attestation itself (`csr register refresh`, "Renew now", and automatically 2 days before expiry on the chain clock of the config cell, checked every 30 minutes, retried after 30 minutes on failure). A refresh needs no stake dep and no consent; only a runner-owned cell is refreshed. Since 0.4.0 the automatic renewal is its own setting, `[registry] auto_renew` (on by default), and runs whether the service runtime is on or off (before, it ran only with the runtime on); the node must be synced.
- `wallet_busy`, `SWEEP_PENDING_AFTER_BLOCKS` and the dashboard password check did not exist in this codebase yet; `wallet::wallet_busy` (refresh pending or keeper submissions pending) and the constant (100 blocks) were added with this change. The dashboard route uses the `wallet_tx` lock and the `wallet_busy` check; the password check applies through the existing session/CSRF middleware.
- The last renewal error is kept in the settings key `renewal_error` and returned by `GET /api/registry`.
## Ping protocol v1.1 (runner)

### D19 A: several monitors
`[[monitors]]` (each with `url` and `pubkey`) replaces the single `[monitor]`, which still works as a one-entry list;
both together are a config error. Default: the network's reference monitor. Inbound accepts a challenge signed by any
configured monitor, the outbound check-in goes to each monitor independently, `csr register attest` and the dashboard's
monitor client use the first entry. The dashboard shows the list; editing stays in the file.

### v1.1 changes
- Fresh check-in: `ts` and `checkin_sig` (`CSR-CHECKIN-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}`) so a copied binding cannot be replayed (F-XC-001). Monitors that do not know them ignore them.
- A challenge with a height above the tip gets a signed answer with empty `block_hash_at_height`; a stale challenge gets 400 `clock_skew`, warned at most every 10 minutes and shown on the Network page (F-XC-004).
- Rejections carry JSON `{"error": "<code>"}`.
- One error table for `POST /v1/attest` in `spec/monitor-public-api.md`, explained by `attest::explain_error` (F-XC-003); `wrong_owner` is gone, `stake_in_use` is 403.
## Node identity – runner

### D6 A: the node's P2P key signs the binding
- Until now the binding (`CSR-BINDING-v1`) was signed by the runner's service key only, so anyone could claim any well-connected node (F-MON-006, F-RUN-007). The runner now reads the CKB node's P2P secret key (`node.p2p_key_file`, read only, kept in memory), checks that it derives the node ID the node reports (`local_node_info.node_id`; peer ID = `base58(0x12 ‖ 0x20 ‖ SHA-256(compressed pubkey))`, tentacle `PeerId::from_public_key`), and posts `CSR-NODE-IDENTITY-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}` signed by the P2P key and the service key to `POST /v1/identity` (`spec/ping-protocol.md` §5b): at start, every 24 h and after a node restart.
- This narrows the rule "CSR never touches the node's data": the key file is only read, never written, chmod-ed or chowned, and only when the operator sets it. A file readable by "other" is refused; on a key/node mismatch no proof is sent (error log, Node page, `csr check`).
- Without the key everything works as before; the monitors will not rank the node once they require the proof. The runner is deployed first: monitors ignore a missing proof until then, and a 404 from an older monitor is not an error.

### D12 A: a daily full-node check for runners behind NAT
- Each monitor publishes the P2P address of its own CKB node (`p2p_address` in `GET /v1/monitor`). The runner asks its node to connect to it (`add_node(peer_id, address)`, Net RPC module) at start and every 6 h unless `get_peers` already lists that peer; the monitor then checks the node among its own peers. "Method not found" means the Net module is off: one warning with the fix, shown on the Node page. No other Net method (`remove_node`, …) is called.

### Phase 1 implementation notes
- One monitor: this code base has no `Config::monitors()` list (the `ping-v1-1-runner` change is not in it), so "every configured monitor" is the single `monitor.url` (or the network default). Results are kept per monitor URL so a list fits later.
- Base58 is implemented in `identity.rs` (about 40 lines, tested) instead of adding `bs58`: the build must work offline, and the crate is not in the local registry.
- No `zeroize` setup exists here (the `csr-dashboard-security` change is not in this code base): the `P2pKey` erases its secret on drop (`SecretKey::non_secure_erase`), and the file buffer and parse buffer are overwritten with zeros. `P2pKey` has no `Serialize`, and its `Debug` prints only the peer ID.
- Identity states: `off`, `proven`, `mismatch`, `unreadable`, plus `unknown` while the key is loaded but the node does not report a node ID (unreachable). While the node is unreachable the dashboard keeps the last known state.
- `add_node` runs only with `ranking.join = true` (it serves the ranking; a non-joined runner does not contact the monitor at all) and does not need the P2P key (the P2P handshake itself proves the peer ID to the monitor's node).
- Retries: a failed proof post (network, 5xx, 200 without `ok`) is retried after 10 minutes, a failed `/v1/monitor` fetch or RPC error for `add_node` too; 4xx rejections and 404 wait for the next regular interval.
- `csr check` exit codes are unchanged (they still reflect R2 only); the identity state is printed and in `--json` as `node_identity`.
- Changing `node.p2p_key_file` in the dashboard settings requires a restart (the key is read at start only).
## D2 A – `OnCellConsumed` is watched by the runner
- **Decision.** A job that should run when a cell is spent is an ordinary `OnDemand` job cell whose payload starts with the 41-byte watch header `"CSRW"` ‖ `01` ‖ out point (36 bytes). No contract change. The runner excludes it (`waiting_for_cell`) until it has seen the cell consumed, then runs it normally. The chain cannot check this, so a runner ignoring the convention could run it early; the standard says so.
- **Watcher.** On every new block the keeper checks the watched out points of the job cells it saw with the same rule (`chain::cell_consumed`: `dead`, or `unknown` with a committed creating transaction that has the output); consumed ones are stored in `watched_cells (outpoint, dead_block)`. Live cells, cells of unknown transactions and RPC errors mean "not yet". Rows no job cell watches any more are removed 1,000 blocks after the cell was found dead. The check lives in the keeper tick (the separate consumed-cell task of `fix-runner-delete-on-consume` is not in this code base).
- **Details.** Headers on triggers other than `OnDemand` are ignored. The run's trigger label is `on-cell-consumed:0x<tx hash>:<index>`. The service gets the whole payload including the header.
## csr-update-check – the mainnet gate is switched on by a release (D5 B)

### D5 B: the gate stays a constant; operators update through a signed release
- `mainnet_runtime_unlocked()` stays a constant in the code. It is switched on by a new release once a person has checked the figures (12 months, 60 diverse runners); a monitor-controlled switch was not chosen, so nobody can unlock mainnet remotely. Every operator then has to update, so updating is made easy now, before it is needed.
- **Check.** `GET /api/update` and `csr update check [--json]` ask `https://api.github.com/repos/ckb-service-runner/ckb-service-runner/releases/latest` (`User-Agent: csr/<version>`, 15 s, at most 1 MiB, cached 1 hour) and answer `{current, latest, newer, notes_url, checked_at, key_configured}`. `csr run` checks once a day (first 60 s after start); the Overview "Update" card shows a notice from the cached result (`update` in `/api/status`), never contacting GitHub on page load. Only the request itself is sent. There is no opt-out setting in phase 1 (one anonymous GET a day).
- **Versions.** Only `X.Y.Z` tags (optional `v`) count; pre-releases and other tags give `newer = null` and are never staged.
- **Client.** `update::http_client` (separate from the redirect-less clients elsewhere): no proxy, at most 5 redirects, each only to an https URL on `github.com`, `api.github.com` or `*.githubusercontent.com` (no explicit port, no user info); every requested URL is checked the same way, and the release-notes link is only shown if it passes that check. Downloads have a 600 s timeout and a 64 MiB limit per file.
- **Signature.** `SHA256SUMS` is signed with ed25519 (`openssl pkeyutl -sign -rawin`, 64 raw bytes; 128 hex characters are accepted too) and checked with `verify_strict` against `RELEASE_PUBKEY`, pinned in the binary. Phase 1 pins all zeros: `check_staged` refuses everything with "no release key configured" and the dashboard hides the Update button. The key is created and its public half pinned by a person in the manual phase; it never enters the repository or AIWorker.
- **Staging.** `POST /api/update/install` and `csr update stage` (refuse without key or newer release) download `csr-<X.Y.Z>-<arch>-linux`, `SHA256SUMS`, `SHA256SUMS.sig` from the asset list of the release, run `check_staged`, and write `csr.new` (0755, set explicitly), `SHA256SUMS`, `SHA256SUMS.sig` and last `ready.json {version, asset}` into `<data root>/<network>/update/` (the network's data directory, next to `csr.db`), each through a temp file and rename. Checks and staging are serialised; a second stage while one runs is refused. Staging always asks GitHub afresh (no 1-hour cache).
- **Password.** `write_needs_password(method, path)` (new in `web/auth.rs`; the list is meant to be merged with the one of `csr-dashboard-security`) lists `/api/update/`: writes there are refused (403) unless a dashboard password is set, because they install software on the host. Without a password, `sudo -u csr csr update stage` does the same. Wallet and registry writes are not added here, to keep existing behaviour.
- **Root installer.** The runner (user `csr`, `ProtectSystem=strict`) cannot replace `/usr/local/bin/csr`. `csr-update.path` (`PathExists=…/update/ready.json`) starts `csr-update.service` (oneshot, root, `PrivateNetwork=yes`) → `/usr/local/lib/csr/install-update.sh <dir>`. The staged files are untrusted (a compromised runner could change them): the script reads them **as the directory's owner** (`runuser`, so planted symlinks cannot make root read or write other files) into a root-only work directory under `/run`, lets the **installed** binary verify the copies (`csr update verify`: signature with its own pinned key, hash, staged version newer than itself, asset name equal to `csr-<version>-<arch>-linux` so the signed checksum line also binds the version), and installs exactly the verified copies (no check/use race). It keeps `csr.prev`, installs with `install -m 0755 -o root -g root` via a temp name and `mv` (atomic), removes the staged files and restarts `csr`; if the new binary's `csr --version` fails it restores `csr.prev` first. A failed verify moves the staged files to `<dir>/rejected/` (so the path unit does not fire again) and logs to the journal.
- **`csr --version`.** The installer needs it, so clap's version flag is enabled (`csr version` stays).
- **Rollback.** Manual: `install -m 0755 /usr/local/bin/csr.prev /usr/local/bin/csr && systemctl restart csr` (docs/install.md). The verify step refuses downgrades, so a rollback never goes through the updater.
- **Release.** `scripts/release.sh <version>` builds, writes `dist/csr-<v>-<arch>-linux`, the tarball and `SHA256SUMS`, and prints the offline signing command; it reads no key. Steps and key handling: `docs/release.md`.
## Contracts v3 – one redeploy before mainnet (D20–D24)

All contract decisions in one redeploy, so operators register again only once (findings F-CON-002 to
F-CON-008). Specs: `spec/registry.md`, `spec/job-cell-standard.md`, `spec/dual-lock.md`.

### D20 A: Registry v3
`RegistryV3 { version = 3, peer_id, service_pubkey, payout_lock_hash, network, stake, expiry_ms,
attestation_sig, endpoint, prekeys }`. The node ID is stored once (no nested attestation copy), and an optional
public `endpoint` (empty or `http(s)://…`, UTF-8, ≤ 128 bytes) lets the monitor call a runner directly
(F-MON-014). `csr-dual-lock` (and the registry's lock check, and the job lock's runner check) accept 32-byte
args for a runner-owned cell (owner = runner) besides `owner ‖ runner`. Data 280 instead of 373 bytes for the
vector entry; with the short args about 125 CKB less per registration. v1/v2 data is error 7 (registry) / 10
(job lock); `Registry` v2 is gone from `csr.mol` and the contracts (the runner keeps its own v2 parser until the
testnet migration).

### D21 A: `CSR-ATTEST-v2`
The monitor signs `blake2b("CSR-ATTEST-v2" ‖ AttestationBodyV2)` with `stake` and `registry_type_hash` (the
registry's own type script hash; the job lock's args). The registry and the job lock both verify it, so an
attestation cannot be used with another deposit or another registry deployment (F-CON-002 on chain, F-CON-005).
A v1 signature fails (10). Error 11 (copy mismatch) is unused since the signature covers the cell's own fields.

### D22 C: stake checked at creation and on changes only
No stake check on a refresh or a run. A withdrawn deposit keeps earning until its attestation expires: at most
7 days as the monitor issues them, 8 days by the cap of D24. Stated in `spec/registry.md` §4.1 and
`spec/job-cell-standard.md` §3.

### D23 A: identity changes need the stake owner's consent
On the owner path a change of `peer_id`, `network` or `service_pubkey` runs the stake rules and the consent
check (the consent text names these fields; F-CON-004). The runner path may change `expiry_ms`,
`attestation_sig`, `endpoint` and `prekeys` only.

### D24 A: lifetime cap and `csr-config-type`
`expiry_ms − clock_ms ≤ 8 days` in `csr-registry` (error 24) and `csr-job-lock` (error 18). The new type script
`csr-config-type` on the monitor config cell (args = Type ID, 32 bytes) keeps Type ID uniqueness
(`ckb_std::type_id`), requires ≥ 41 bytes of data, lets `clock_ms` only grow or stay, never lets
`monitor_pubkey` change and refuses destruction (errors 5–10, `spec/registry.md` §1; F-CON-006).

### Contracts v3 Phase 1 – contracts
- **Registry args = `config_id`, config found by computed type hash.** "Found by the type hash of
  `csr-config-type` with the configured args": the registry's type args are now the config cell's
  `csr-config-type` args (its Type ID), and `csr-registry` / `csr-job-lock` compute the config type hash from
  the embedded `csr-config-type` code hash (`build.rs`, like the dual lock's), hash type `data2`, and these
  args. So a registry deployment can only ever use a config cell governed by the real `csr-config-type`;
  `config_type_hash` as an opaque arg (v2) could name any Type ID cell.
- **`csr-config-type` uses hash type `data2`.** `ckb_std::type_id` needs the `type-id` feature, which pulls
  `ckb-types` and the `bytes` crate; its atomics compile to RISC-V A instructions. `-C target-feature=-a` (the
  ckb-std README's alternative) no longer compiles with the current LLVM ("Cannot select AtomicLoadAdd"), and
  ckb-vm 0.24 has the A extension in VM version 2. So the contract is built normally and referenced with
  `data2`; the registry and the job lock accept only `data2` for it. The other contracts keep avoiding
  `ckb-types`.
- **Shared code `csr-registry/src/attest.rs`** (RegistryV3 parser, v2 message, recovery, cap, config type hash,
  dual-lock args) is used by the registry, the job lock (`#[path]`) and the contract tests, like `consent.rs`.
- **`registry_v3_data_matches_vector` "inside the contract".** The vector's `registry_type_hash` (`0x4e…4e`) can
  be no real script hash, so the vector's own signature cannot pass the VM's `load_script_hash`. The test (1)
  builds both data forms with the test builder and compares them byte for byte, (2) runs the contract's own
  `attest.rs` (parser, message, recovery, check) on the vector bytes – the signature recovers to the vector's
  monitor key – and (3) registers, in the VM, the vector entry signed by the vector's monitor secret under the
  test deployment's registry type hash.
- **Job lock checks the attestation signature** (step 4 of the task): registry v3 dep, signature over
  `CSR-ATTEST-v2` with its own args as `registry_type_hash` (failure → 10, the "registry dep invalid" code; no
  new code), expiry (17), cap (18). Cost: one more recovery, ~9.6M cycles per settlement instead of ~1.7M.
- **Same index (F-CON-003, error 19).** The job input's index *i* is the index of the only input with the job
  lock's own lock hash. Recreation: the existing "exactly one" check (12) runs first, then the index (19).
  Refund: the net inflow check (13) runs first, then output *i* must have the refund lock and on its own at
  least the remainder (19) – a dust refund at *i* with the rest elsewhere would otherwise still let two jobs
  share. The runner's settlement layout `[runner wallet, job] → [wallet + tip, job | refund]` already puts the
  job's output at the job's index (1).
- **Second job-lock build for the tests.** `scripts/build-contracts.sh` builds `csr-job-lock` a second time with
  `CSR_JOB_LOCK_VARIANT=test-variant` (an embedded constant kept in the binary with `black_box`) into
  `contracts/build/test/csr-job-lock-variant` (never deployed); `job_two_lock_versions_share_refund_rejected`
  deploys it next to the normal build.
- **Contract tests build the contracts if missing.** `csr-contract-tests/build.rs` runs
  `scripts/build-contracts.sh` (into `contracts/build/cargo-target`, outer cargo variables removed) only when a
  binary under `contracts/build/` is missing, so the suite also runs on a fresh checkout; the binaries are
  byte-identical to a direct script run. The script honours `CARGO_TARGET_DIR`.
- **Error mapping of the adapted v2 tests.** Attestations for another key, peer id or network, and the copied
  attestation of F1 now fail with 10 (signature) instead of 11 / 17. The owner's service-key rotation now needs
  the stake dep (D23 A; consent by the owner's own input).
- **F-CON-007** is documented, not changed: one job input or output per service group per transaction
  (`csr-job-type` error 9); batching jobs of one service in one transaction is not supported.
- **Runner and `csr-dev` untouched in this phase.** `deploy-contracts` still deploys four contracts and the
  runner still writes v2 data; both follow in the next phases (the deployments need a new config cell with
  `csr-config-type`, so `--keep-config` cannot be reused for v3).

### Contracts v3 Phase 2 – runner and devtools
- **v2 and v3 deployments side by side.** `deployments/testnet.json` is still the v2 testnet deployment (the
  testnet redeploy needs the testnet key and is not part of this phase). A deployments file is v3 when it has
  `scripts.config_type`; then the config cell is looked up under `csr-config-type` (`data2`, args =
  `config_cell.type_args`), the registry's args are the `config_id`, and the runner checks that
  `config_cell.type_hash` and `registry_type_hash` follow from the scripts (a mismatch is a load error). Without
  it (v2) the runner keeps the Type ID lookup and the v2 registry args so that existing v2 registrations keep
  running jobs until the migration, but `register create` refuses: the runner writes v3 data only.
- **No separate `register_refresh`.** This branch has no `csr-attestation-renewal` code; a refresh is a second
  `register create` (new attestation, same or new deposit, current `ranking.public_endpoint`), which updates
  the runner-owned cell on the owner path. The stake dep and consent are sent anyway (harmless on a refresh).
- **Runner-owned = 32-byte args, 64-byte form still recognised.** New cells use `runner_lock_hash` alone;
  `register create`/`remove` also accept a v3 cell whose args are `runner ‖ runner` and rewrite it to 32 bytes
  on the next update (the owner path may change the lock).
- **Monitor's v2 reply.** `attest::check` expects the v1 fields plus `stake` (`0x<tx>:<index>`) and
  `registry_type_hash` (`0x` + 64 hex) and verifies the signature over `CSR-ATTEST-v2` with the *expected*
  stake and registry type hash (the runner's own deployment), so a relabelled reply fails. A reply without
  `stake` is reported as a v1 attestation. The request body is unchanged (the monitor knows its registry; no new
  field is sent, to stay compatible with a strict request parser).
- **Checks before broadcasting.** `register create` also verifies the 8-day cap against the on-chain clock and
  recovers the attestation signature against the config cell's `monitor_pubkey`, so a wrong attestation is a
  clear error instead of a script error from the node.
- **`ranking.public_endpoint` validation** is a bit stricter than the contract: empty, or `http(s)://host[:port]`
  with a host, without user info, path, query, fragment or trailing slash, at most 128 bytes.
- **`build_run`** already had the layout `[wallet, job] → [wallet + tip, job | refund, skeleton…]`; it is now
  explicit (`JOB_INDEX = 1`, unit test `run_output_sits_at_the_job_input_index`).
- **`csr-dev`.** `Out.type_id` with a `type_` fills that script's args with the Type ID (used for the
  `csr-config-type` config cell; the code cell is a dep of the creating transaction). `--keep-config` refuses a
  v2 file and a config cell whose `csr-config-type` code hash differs from this build's binary (the registry
  and the job lock embed it). `clock-update` uses `max(tip timestamp, current clock_ms)`. `attest` needs
  `--stake` and takes `registry_type_hash` from the deployments file (refuses a v2 file); a unit test checks the
  message against the shared vector. The A8 devnet acceptance scripts (`acceptance/a8/…`) still call the v1
  `attest` and are obsolete for v3; the dashboard setup text keeps "about 700 CKB" (still enough for the
  ~420 CKB v3 cell).


## Implementation notes (formerly the root DECISIONS.md)

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

## Dashboard security, phase 1
- See `docs/decisions.md` ("Dashboard security, phase 1"): password required for money/registration/service/settings writes, Host/Origin checks, login limiter, split session pools, Secure cookie behind HTTPS.
- Dashboard notice: a banner on the Wallet and Settings pages (registration lives on the Wallet page) while no password is set, and on any `password_required` refusal, linking to the Settings password note.
See docs/decisions.md, "Sandbox storage, phase 1".
## Attestation renewal (D3 A)
- The runner renews the registry attestation itself (`csr register refresh`, "Renew now", and automatically 2 days before expiry on the chain clock of the config cell, checked every 30 minutes, retried after 30 minutes on failure). A refresh needs no stake dep and no consent; only a runner-owned cell is refreshed.
- `wallet_busy`, `SWEEP_PENDING_AFTER_BLOCKS` and the dashboard password check did not exist in this codebase yet; `wallet::wallet_busy` (refresh pending or keeper submissions pending) and the constant (100 blocks) were added with this change. The dashboard route uses the `wallet_tx` lock and the `wallet_busy` check; the password check applies through the existing session/CSRF middleware.
- The last renewal error is kept in the settings key `renewal_error` and returned by `GET /api/registry`.
## ping-v1-1-runner
- Empty `url`/`pubkey` of a `[[monitors]]` entry take the network default, like the old `[monitor]` fields.
- An entry with an unusable key is skipped with a warning on the inbound side; `attest` uses the first entry as given.
- The 4096-byte body limit is checked in the handler (axum limit raised to 64 KiB) so `too_large` has a JSON body.
- The Network page shows a clock-skew warning via `ping.clock_skew_at` of `GET /api/network`.
- The monitor repository's spec and `docs/PROTOCOL.md` are outside this directory and not changed here.
## Node identity – runner, Phase 1

See `docs/decisions.md`, section "Node identity – runner" (D6 A, D12 A and the "Phase 1 implementation notes"): single configured monitor (no `Config::monitors()` here), own base58 instead of `bs58` (offline build), erase-on-drop instead of a `zeroize` setup that does not exist here, the extra `unknown` identity state, `add_node` gated on `ranking.join` only, 10-minute retries after failures.
csr-update-check phase 1: see `docs/decisions.md`, section "csr-update-check – the mainnet gate is switched on by a release (D5 B)".
Contracts v3, phase 1: see `docs/decisions.md`, section "Contracts v3 – one redeploy before mainnet (D20–D24)", subsection "Contracts v3 Phase 1 – contracts" (registry args = config Type ID with the config type hash computed from the embedded `csr-config-type` code hash; `csr-config-type` referenced with hash type data2 because `ckb_std::type_id` brings A-extension atomics; how the shared vector is checked "inside the contract"; job lock signature failures map to error 10; order of the same-index checks; the test-only job-lock variant build; the test crate's fallback `build.rs`).

Contracts v3, phase 2: see `docs/decisions.md`, subsection "Contracts v3 Phase 2 – runner and devtools" (v2/v3 deployments side by side with `register create` refused on v2; no separate refresh command; 64-byte runner-owned args still recognised; format of the monitor's v2 reply; checks before broadcasting; endpoint validation; `csr-dev` `--keep-config` and clock rules).

Contracts v3, phase 2 (retry): the previous run failed only because `acceptance/aiworker-contracts-v3/registry_v3_tests.rs` had been deleted; the runner/devtools work was kept as is, and the files under `acceptance/aiworker-contracts-v3/` are left untouched (`chain.rs` only includes the test module).
## Documents (csr-docs)
- First transfer: 700 CKB, stated once in README "Runner wallet, stake and registration" (539 CKB registry cell + fee + ≥61 CKB change ≈ 601, with headroom). The v3 contract is not in this repo; the figure uses the layout in `spec/registry.md`.
- `identity proven` in the dashboard: `identity_proven` from the monitor if present, else `check_type == p2p_verified`.
- `deployments/testnet.json`: `rpc` and `anyone_can_pay` removed (nothing in the runner reads them; csr-dev falls back to its constant ACP hash).
- Milestone history: index in `docs/milestones/README.md`; old URLs in `docs/spec/` and `.github/` changed to `ckb-service-runner/ckb-service-runner`; the owner name was dropped from the CRT plan.
- A7 history in specs is kept between `<!-- history -->` markers.

## Mainnet dashboard release (D29–D39, 31 Oct 2026)

Taken by the project owner. CSR goes to mainnet now as a **node dashboard only**; the Service Runtime stays
testnet-only until the Phase 2 gate below is met. Work items: AIWorker specs `mon-rank-without-stake` and
`csr-mainnet-dashboard`.

- **D29 – Ranking never needs a stake.** Every runner that joins the ranking is ranked, on every network,
  registered or not. A registration (DAO stake + registry cell) is needed **only to activate the Service Runtime**
  (job assignment). The monitor keeps publishing the stake as `stake_status` and sets a `FLAG_STAKED` bit in the
  epoch leaves, so apps (CRT) pick only staked runners for jobs. Replaces the stake-only ranking of
  fix-monitor-stake-ranking (registry mode).
- **D30 – Node identity proof stays mandatory for ranking** (P2P key, `require_identity_proof`).
- **D31 – Network is auto-detected, no toggle.** The installer reads the chain of each local node and writes
  `network` itself; the dashboard shows the network but cannot change it. A machine running a testnet and a
  mainnet node gets **both instances** installed.
- **D32 – Two instances per machine are supported**: `csr@testnet` and `csr@mainnet` (systemd template), one
  shared data root (`/var/lib/csr/<network>/` is already separate per network), configs `csr-<network>.toml`,
  dashboards 8130 (testnet) and 8140 (mainnet), ping listeners 8131 and 8141.
- **D33 – Mainnet monitor**: its own instance on pswsl beside the testnet monitor, reached through the Strato
  VPS like testnet, with its own signing key.
- **D34 – Mainnet monitor URL and public key ship in `config/networks/mainnet.toml`** with the release.
- **D35 – Versions**: the mainnet dashboard ships as v0.x, the same version line as testnet. **v1.x is reserved
  for the release that unlocks the mainnet runtime** (D5 B: the gate is switched on by a signed release).
- **D36 – Phase 2 gate** (mainnet runtime): at least **4 months** (120 days) of monitor history and **≥ 20
  ranked runners**, with the existing diversity limits (largest ASN share ≤ 25 %, largest country share ≤ 50 %).
  Was 12 months / 60 runners. Monitor `gate_min_days = 120`, `gate_min_runners = 20`.
- **D37 – `MIN_STAKE` stays 10,000 CKB on testnet and mainnet** (closes open point (1) of D-A7-1).
- **D38 – External contract review**: decided later, before the 1.x release.
- **D39 – Testnet tester support**: no job simulator on mainnet; mainnet carries real use only. Whether to
  sponsor the testnet stake of the first testers is still open.
