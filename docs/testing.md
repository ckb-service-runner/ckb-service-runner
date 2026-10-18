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
