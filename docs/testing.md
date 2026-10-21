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

`target/debug/csr-dev [--rpc URL] [--json] <deploy-contracts|clock-update|attest|fund|dao-deposit|sign-tx|sign-message|reclaim|owner-lock-args|deploy-service|job>` – devnet/testnet only (refuses other chains; testnet needs `CSR_DEV_TESTNET_KEY`). Run `bash scripts/build-contracts.sh` first; `deploy-contracts` writes `deployments/devnet.json` (the four CSR contracts, the monitor config cell and, on the devnet, the vendored anyone-can-pay lock `devtools/vendor/anyone_can_pay`; `--keep-config EXISTING` reuses the config cell of another deployments file).  `dao-deposit --ckb N [--key genesis|0x<key>]` makes a Nervos DAO deposit (output 0). `sign-tx` signs every secp256k1 and anyone-can-pay lock group of the key. `sign-message --text T [--key genesis|0x<key>]` signs like Neuron's "Sign message" (`{signature, lock_args}`; the stake consent). `reclaim --outpoint 0x<tx>:<i> [--outpoint …]` spends plain cells of the key's own lock back to it (refuses typed cells and other locks). `deploy-contracts --keep-config EXISTING --reuse-unchanged` keeps every contract whose binary is unchanged and whose code cell is live. Tests: `node --test --test-concurrency=1 acceptance/a8b/p3.test.mjs` (devnet running).

### Operator flow on devnet (registration end to end)

With the devnet running and `deployments/devnet.json` written (`csr-dev deploy-contracts`), and a debug `csr` with `network = "devnet"`, `[devnet] deployments = "deployments/devnet.json"`:

```
csr identity --json                                   # service_pubkey, runner_lock_args, runner_address
csr-dev fund --lock-args <runner_lock_args> --ckb 700 # the runner wallet; registry cell ≈ 540 CKB + 61 CKB change (700 or more)
csr-dev dao-deposit --ckb 10000                       # the stake: {tx_hash, index} of a DAO deposit of the genesis key
csr register consent-message --stake 0x<tx>:<index> --json         # the text the stake owner signs
csr-dev sign-message --text '<message>' --key genesis               # = "Sign message" in Neuron
csr-dev attest --peer-id <node id> --service-pubkey <pubkey> --network devnet --out att.json
csr register create --stake 0x<tx>:<index> --consent 0x<signature> --attestation att.json --json
csr register status --json
csr wallet status --json
csr register remove --json                           # the registry capacity returns to the wallet
csr wallet send --to <ckt1… address> --ckb 100 --json # by hand
csr wallet sweep --json                               # one sweep now (needs [payout] sweep_address, sweep_threshold_ckb, sweep_amount_ckb)
```

`register create` refuses (in this order) without a Synced R2, for an attestation whose public key or network differs from this node's (or that is already expired by the on-chain clock), for a bad stake (malformed out point, not a live cell, not a Nervos DAO deposit, below 10,000 CKB, not under a standard secp256k1 lock), for a consent that does not recover to the stake owner (the text is rebuilt with the attestation's peer id), and when the runner wallet cannot pay the cell, the fee and the 61 CKB change (the message names the runner address). The registry cell is output 0 with dual lock `runner ‖ runner`, the stake is `cell_deps[0]`, and witness 0 carries the runner's signature and the consent (`output_type`). A second `register create` with a new deposit updates the same cell. Mine blocks (`scripts/devnet.sh mine N`) if a transaction stays pending. Tests: `node --test --test-concurrency=1 acceptance/a8/p3.test.mjs acceptance/a8/p4.test.mjs`.

### Keeper jobs and the race test (A3c)

`node --test --test-concurrency=1 acceptance/a3c/p2.test.mjs acceptance/a3c/p3.test.mjs` (devnet running, `bash scripts/build-services.sh` done). `p2`: an `every:2` job runs three times, the tips are settled (since A8 they go into the runner wallet, `acceptance/a8/p4.test.mjs`). `p3`: two CSR instances with separate data roots share the devnet node and race for one job; exactly one settles, the other ends in `Lost` at no cost. Manual demo steps: `docs/milestones/A3c.md`.

### Manual testnet runbook "with Neuron" (not run by the test suite)

Prerequisites: a synced testnet CKB node (full node with the Indexer module), a funded testnet key, the built contracts.

1. **Deployment.** The testnet deployment is embedded in the runner (`deployments/testnet.json`, compiled in) and `config/networks/testnet.toml` carries the real monitor; nothing to configure. (Developers redeploy with `bash scripts/build-contracts.sh`, free the old cells with `csr-dev reclaim --outpoint 0x<tx>:<i> …` and run `csr-dev deploy-contracts --keep-config deployments/testnet.json --reuse-unchanged --out deployments/testnet.json`.)
2. **Clock.** The monitor `monitor-testnet.vericell.net` keeps the on-chain clock fresh; no `csr-dev clock-update` is needed.
3. **Runner wallet.** `csr identity --json` (network `testnet`) shows `runner_address`; send about 700 CKB there from any wallet (the registry cell takes ≈ 540 CKB, 61 CKB stay as change).
4. **Sweep (optional).** Set `[payout] sweep_address` to your own wallet's address; the runner sends `sweep_amount_ckb` there whenever the wallet reaches `sweep_threshold_ckb`.
5. **Stake.** Deposit at least 10,000 CKB into the Nervos DAO with your wallet (Neuron or `ckb-cli dao deposit`); note the deposit's out point `0x<tx hash>:<index>`. CSR never moves it; keep it deposited while registered.
6. **Consent.** `csr register consent-message --stake 0x<tx>:<index>` prints the one-line text; sign it with the deposit's wallet ("Sign message" in Neuron) and check with `csr register verify-consent --message '<text>' --signature 0x…`.
7. **Register.** `csr register create --stake 0x<tx>:<index> --consent 0x<signature>` asks the real testnet monitor for the attestation (it needs `[ranking] join = true` and successful checks in the last 24 hours, otherwise `not_eligible`), then builds, signs and submits the registration; `csr register status --json` shows `registered: true`.
8. **Payout.** Tips go into the runner wallet; `csr wallet send --to <address> --ckb N` moves CKB out by hand.

In the dashboard (`#/wallet`, "Wallet & registration") the same steps are: copy the runner address, send the CKB from Neuron, enter the stake out point, "Show text to sign", sign it in Neuron (Tools → Sign/Verify Message, with the deposit's address), paste the signature and press "Register"; set `payout.sweep_address` (Settings) to your Neuron address.
