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
