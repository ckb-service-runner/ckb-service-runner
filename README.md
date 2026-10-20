# CSR – CKB Service Runner

CSR is a Rust side application that runs next to a fully synced [CKB](https://github.com/nervosnetwork/ckb) full node.
Milestone **A1** delivers the `csr` binary with:

- node monitoring and the **R2 sync guard** (is the node synced, connected and indexed?),
- a resource monitor (CPU, RAM, disk),
- an **embedded web dashboard** (Overview, Node, Resources, Settings, Logs) served by `csr` itself – no separate web server, no build step,
- dashboard authentication (argon2id password, sessions, CSRF protection),
- the **ping responder** (inbound challenge endpoint and outbound NAT check-in) the CSR Network Monitor uses to rank nodes.

CSR never sends a transaction in A1 and never edits your `ckb.toml`.

## Install from source

```sh
. "$HOME/.cargo/env"
cargo build --release
./target/release/csr version
```

## Quick start

```sh
csr detect            # inspect the configured node (add --probe to try 127.0.0.1:8114 and :8124)
csr check             # one R2 check; exit 0 = Synced, 2 = not Synced / unreachable
csr set-password      # optional: protect the dashboard
csr run               # start the service (testnet by default); Ctrl-C / SIGTERM stops it
```

`csr detect` tells you if the node is reachable, which chain it serves, and whether the Indexer module is enabled.
If it is not, add `"Indexer"` to `modules` under `[rpc]` in `ckb.toml` yourself and restart the node.

Global options: `--data-root DIR` (default `~/.csr`), `--config FILE` (default `<data-root>/csr.toml`; built-in defaults if absent),
`--network testnet|mainnet`. Mainnet needs `csr run --confirm-mainnet` once.

## Configuration

Copy `config/csr.example.toml` to `~/.csr/csr.toml`. Every key is optional, unknown keys are errors, and `csr config validate`
checks a file without creating anything. The main sections: `[node]` (RPC URL, loopback only), `[dashboard]` (bind address, password hash),
`[ranking]` (join, ping mode and port), `[monitor]`, `[runtime]`, `[limits]`, `[storage]`, `[logging]`.
Per-network state lives in `<data-root>/<network>/` (keys, SQLite database, logs).

## Dashboard

Open `http://127.0.0.1:8130/` after `csr run`. Pages: `#/overview`, `#/node`, `#/network`, `#/resources`, `#/settings`, `#/logs`.
A non-loopback bind requires a password (`csr set-password`). All writes need the session cookie and the CSRF token.
The JSON API (`/api/status`, `/api/node`, `/api/resources`, `/api/settings`, `/api/events`, …) is documented in `docs/milestones/A1.md`.

The **CSR monitor** page (`#/network`) shows a world map of the CSR network (2° areas, from the monitor's `/v1/locations`), nodes worldwide/reachable, a per-country bar list, your rank, availability (7/30/365 d), check type and attestation,
fed by the monitor's public API (`spec/monitor-public-api.md`, cached 5 min; stale data is flagged). See `docs/milestones/A1plus.md`.

## Ranking (ping responder)

With `ranking.join = true` the CSR Network Monitor can verify that your node is online, synced and bound to your service key.

- `ping_mode = "inbound"`: CSR listens on `ranking.ping_bind` (default `0.0.0.0:8131`) and answers signed challenges at `POST /v1/challenge`.
- `ping_mode = "outbound"`: for nodes behind NAT. CSR checks in with the monitor every 10 minutes and answers the challenge it gets back. The ping port stays closed.
- `ping_mode = "both"`: both.

Protocol details: `spec/ping-protocol.md`. The monitor URL and key are placeholders until the real monitor exists (see `docs/open-questions.md`).

## Services (A2)
Build the example services with `bash scripts/build-services.sh`, then
`csr service run-local --manifest services/manifests/hello-rust.toml --payload 0x0102`.
Register with `csr service add-local --manifest PATH`, manage with `csr service list|approve|revoke|enable|disable`
or on the dashboard page `#/services`. Host interface: `spec/host-interface.md`. Details: `docs/milestones/A2.md`.

## Jobs and earnings (A3c)
With `runtime.enabled = true`, `csr run` discovers keeper job cells on the chain, runs approved on-chain services (`csr service add-onchain`), submits the job transactions and tracks settlement. `csr jobs plan --json` shows the selection. Dashboard pages `#/jobs` and `#/earnings`. Demo: `docs/milestones/A3c.md`.

## Stake and payout (A7)
A registration references a **stake**: a Nervos DAO deposit of at least 10,000 CKB that the operator makes in their own wallet (it keeps earning DAO compensation; CSR never moves it). Tips go to the operator's **payout cell**, a standard anyone-can-pay cell (`[payout] lock_args`). Commands: `csr payout status|prepare-create|prepare-withdraw`, `csr register attest|prepare --stake 0x<tx>:<index>`; dashboard page `#/payout` ("Stake & payout"). Details: `docs/milestones/A7.md`, `spec/registry.md`.

## Roadmap

| Milestone | Content |
|---|---|
| **A1** | Runner, R2 guard, resource monitor, dashboard, auth, ping responder (this release) |
| **A2** | Service runtime: CKB-VM sandbox, host interface, encrypted storage, local runs, Rust/C/JS example services, Services page (`docs/milestones/A2.md`) |
| A3 | Services, job market and on-chain interaction |
| A4 | Production hardening, mainnet unlock via the monitor |

See `docs/spec/` for the full specification.
