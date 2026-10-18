# Testing

## Local devnet (`scripts/devnet.sh`)

A throw-away single-node CKB dev chain for the A3b tests and for `csr-dev`.

| Command | Behaviour |
|---|---|
| `start` | Downloads the official `ckb` v0.210.0 (linux x86_64, SHA-256 pinned and checked) into `.tools/` if missing; on first use runs `ckb init --chain dev` in `.devnet/` and patches the config (RPC `127.0.0.1:18114`, P2P `127.0.0.1:18115` on loopback only, no bootnodes, RPC modules incl. `Indexer` and `IntegrationTest`, block assembler for the devnet miner key). Starts `ckb run` in the background (`.devnet/ckb.pid`, log `.devnet/ckb.log`), waits for the RPC, mines one block so the node leaves IBD. Already running → exit 0. Port 18114/18115 held by something else → exit 1. |
| `mine N` | `generate_block` N times. |
| `status` | exit 0 if running, 3 if not. |
| `stop` | stops the node; no error if not running. |
| `reset` | `stop` + delete `.devnet/`. |

`.tools/`, `.devnet/` and `deployments/devnet.json` are gitignored. The harness never touches ports other than 18114/18115. Keep only one devnet node running (~200–300 MB RAM).

### Keys

`devtools/devnet-keys.json` holds **public, well-known test keys – devnet only, never use them anywhere else**:

- `genesis`: dev-chain key from the ckb repo's `specs/dev.toml` (holds 20 billion CKB at genesis); the funding / "operator wallet" account.
- `miner`: second well-known dev key, used as block assembler (receives block rewards).
- `monitor`: devnet monitor key (invented for CSR tests).

### Running the tests

```
bash scripts/devnet.sh reset && bash scripts/devnet.sh start
node --test --test-concurrency=1 acceptance/a3b/p1.test.mjs
bash scripts/devnet.sh stop
```

### `csr-dev`

`target/debug/csr-dev [--rpc URL] [--json] <deploy-contracts|clock-update|attest|fund|sign-tx|owner-lock-args>` – devnet/testnet only (refuses other chains; testnet needs `CSR_DEV_TESTNET_KEY`). Run `bash scripts/build-contracts.sh` first; `deploy-contracts` writes `deployments/devnet.json`. Tests: `node --test --test-concurrency=1 acceptance/a3b/p2.test.mjs` (devnet running).

### Operator flow on devnet (registration end to end)

With the devnet running and `deployments/devnet.json` written (`csr-dev deploy-contracts`), and a debug `csr` with `network = "devnet"`, `[devnet] deployments = "deployments/devnet.json"`:

```
csr identity --json                                   # service_pubkey, runner_lock_args
csr-dev fund --lock-args <runner_lock_args> --ckb 200 # runner key cell
csr-dev owner-lock-args                               # the "operator wallet" (genesis account)
csr float prepare-create --ckb 1000 --owner-lock-args <args> --out float.json
csr-dev sign-tx --in float.json --out float.signed.json --key genesis
csr tx submit float.signed.json --json
csr-dev attest --peer-id <peer id> --service-pubkey <pubkey> --network devnet --out att.json
csr register prepare --attestation att.json --owner-lock-args <args> --out reg.json
csr-dev sign-tx --in reg.json --out reg.signed.json --key genesis   # owner input only
csr tx submit reg.signed.json --json
csr register status --json
```

`register prepare` refuses without a Synced R2, without a float, without a runner key cell, and for an attestation whose public key or network differs from this node's (or that is already expired by the on-chain clock). It signs the runner key cell input itself with the service key. Mine blocks (`scripts/devnet.sh mine N`) if a transaction stays pending. Test: `node --test --test-concurrency=1 acceptance/a3b/p4.test.mjs`.

### Keeper jobs and the race test (A3c)

`node --test --test-concurrency=1 acceptance/a3c/p2.test.mjs acceptance/a3c/p3.test.mjs` (devnet running, `bash scripts/build-services.sh` done). `p2`: an `every:2` job runs three times, the float grows by exactly the tips. `p3`: two CSR instances with separate data roots share the devnet node and race for one job; exactly one settles, the other ends in `Lost` at no cost. Manual demo steps: `docs/milestones/A3c.md`.

### Manual testnet runbook (not run by the test suite)

Prerequisites: a synced testnet CKB node (full node with the Indexer module), a funded testnet key, the built contracts.

1. **Deploy the scripts (developer).** `bash scripts/build-contracts.sh`, then
   `CSR_DEV_TESTNET_KEY=0x<privkey> target/debug/csr-dev --rpc http://127.0.0.1:<testnet rpc port> deploy-contracts --out deployments/testnet.json`.
   `csr-dev` refuses any chain except `ckb_dev` and `ckb_testnet`. The key pays for the code cells and acts as the monitor key; keep the deployments file.
   Copy the resulting hashes into `config/networks/testnet.toml` (TODO: shipped values do not exist yet; until then `csr` chain commands fail with "scripts not deployed").
2. **Keep the clock fresh.** `csr-dev clock-update` regularly (the real monitor does this in production).
3. **Identity.** `csr identity --json` (network `testnet`), fund `runner_lock_args` with ≥ 100 CKB from any wallet.
4. **Float.** `csr float prepare-create --ckb N --owner-lock-args <your wallet lock args> --out float.json`. Open `float.json` for signing:
   - ckb-cli: `ckb-cli tx sign-inputs --tx-file float.json --privkey-path <key> --add-signatures` (or `ckb-cli wallet`-based signing), then `ckb-cli tx send --tx-file float.json`; or
   - Neuron: offline signing import of the same file (see `docs/open-questions.md`: acceptance is unverified).
   Alternatively submit with `csr tx submit float.signed.json`.
5. **Attestation.** `csr-dev attest --peer-id <node peer id> --service-pubkey <pubkey> --network testnet --out att.json` (testnet monitor stand-in).
6. **Register.** `csr register prepare --attestation att.json --owner-lock-args <args> --out reg.json`; the owner signs only the owner input (the runner key cell input is already signed by CSR); submit; `csr register status --json` shows `registered: true`.
7. **Top up / withdraw.** `csr float prepare-topup|prepare-withdraw …` and the same sign/submit steps, or the dashboard Float page buttons (Create, Top up, Withdraw, Register, upload signed file).
