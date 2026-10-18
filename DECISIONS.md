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
