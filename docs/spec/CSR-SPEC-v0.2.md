# CKB Service Runner (CSR) — Implementation Specification v0.2

**Repository:** https://github.com/psawyerberlin/ckb-service-runner
**Audience:** Claude Code (implementer), Patrick (reviewer)
**Status:** Draft — 2026-10-18 (replaces v0.1)
**Companion documents:** `CSR_CRT_UseCase_OverallDeployment_Plan.md` (overall plan), CSR Network Monitor spec (separate, to follow)

---

## 0. Instructions for Claude Code

1. **Testnet first.** Code, examples, tests and docs default to testnet. Never send a mainnet transaction from any test, example or script.
2. **Never touch the CKB node.** Do not modify the `ckb` binary, its `ckb.toml` or its data directory. Talk to it over local RPC only (plus, later, P2P for identity).
3. **Verify every CKB RPC method and field** against the docs for ckb 0.210.x before using it. If something in this spec does not exist, stop, write it into `docs/open-questions.md`, and continue with the next task.
4. **Build in the milestone order of section 14.** Each milestone ends with `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`, a `CHANGELOG.md` entry, and a short demo note in `docs/milestones/<id>.md`.
5. **Runtime dependencies: none beyond the binary.** No Node.js, no database server, no external CDN. The dashboard is plain HTML/CSS/JS embedded in the binary. The Rust dependency tree will be large (ckb-* crates); that is accepted. Pin crate versions in `Cargo.lock`.
6. **Pin `ckb-vm` to the version used by the node's `ckb-script` crate** for ckb 0.210.x, so services behave like on-chain scripts.
7. **Security-relevant code gets tests including failure cases:** R2 guard, sandbox limits, key handling, float lock, registry, ping signatures, settings auth.
8. When something is ambiguous, choose the simpler option, implement it, and record it in `docs/decisions.md`.
9. Mocks are required for the CSR Network Monitor (it is built in parallel). Put the mock in `runner/tests/mock_monitor/`.

---

## 1. What CSR is

CSR is a **side application** on the same PC as a fully synced CKB full node (standalone `ckb`, or Neuron in full-node mode). It:

1. Shows a **lightweight node dashboard** (sync, metrics, resources, network ranking, jobs, earnings, settings).
2. **Answers pings** from the CSR Network Monitor, or initiates them when behind NAT, so the node is ranked.
3. **Runs services** — RISC-V programs stored in cells, executed in CKB-VM — **timed or on demand**, for tips paid by the consumer.
4. **Manages the operator's float cell**, which pays transaction fees and collects tips.

### Phases (from the deployment plan)

| Phase | Mainnet | Testnet |
|---|---|---|
| 1 (first 6 months) | dashboard + ranking only; runtime visible but inactive | dashboard + ranking + runtime fully active |
| 2 (after 6 months and enough diverse nodes) | runtime active | runtime active |

v0.2 implements Phase 1 behaviour. The mainnet gate is a function `mainnet_runtime_unlocked() -> bool` returning `false`, with a TODO to read the activation status from the monitor's stat cell later. A debug-only flag `--dev-unlock-mainnet-runtime` exists only in debug builds.

---

## 2. Architecture

```
┌────────────────────────────── operator's PC ───────────────────────────────┐
│ ┌──────────────┐  RPC 127.0.0.1  ┌──────────────────────────────────────┐   │
│ │ CKB full node│◄───────────────►│ csr                                  │   │
│ │ (ckb/Neuron) │                 │  node monitor + R2 guard             │   │
│ └──────────────┘                 │  resource monitor                    │   │
│                                  │  ping responder / NAT check-in  ─────┼───┼──► CSR Network Monitor
│                                  │  job discovery + scheduler           │   │     (public API)
│                                  │  CKB-VM sandbox + host interface     │   │
│                                  │  service storage (encrypted)         │   │
│                                  │  tx builder: jobs, float, registry   │   │
│                                  │  state DB (SQLite, embedded)         │   │
│                                  │  HTTP: dashboard + API  :8130 (local)│   │
│                                  │  HTTP: ping endpoint    :8131 (public)│  │
│                                  └──────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Suggested crates

| Purpose | Crate |
|---|---|
| Async, HTTP server/client | `tokio`, `axum`, `reqwest` (rustls) |
| Static asset embedding | `rust-embed` |
| CKB RPC + tx building | `ckb-sdk` (v5), `ckb-types`, `ckb-hash`, `ckb-jsonrpc-types` |
| VM | `ckb-vm` (pinned, see §0.6) |
| State DB | `rusqlite` (bundled) |
| System metrics | `sysinfo` |
| Config | `serde`, `toml` |
| Logging | `tracing` |
| Signing | `secp256k1` |
| Storage encryption | `chacha20poly1305` |

### Repository layout

```
runner/            csr binary (Rust)
  src/
    main.rs  config.rs  network.rs  db.rs  identity.rs
    node/        RPC client, node monitor, R2 guard
    resources.rs
    ping/        inbound responder, outbound check-in, challenge answers
    monitor/     client for the Network Monitor public API
    runtime/     CKB-VM sandbox, host interface, limits
    storage/     per-service encrypted KV
    jobs/        discovery, selection, scheduler, lifecycle
    chain/       tx building: jobs, float, registry; submission; earnings
    web/         axum routes, API, SSE
  assets/        dashboard HTML/CSS/JS
  tests/mock_monitor/
contracts/         on-chain scripts (Rust + ckb-std): registry, float-lock, job
services/          example services: hello (Rust, C, JS), heartbeat, crowdcell-finalize
devtools/          csr-dev CLI (testnet/devnet only)
spec/              job-cell-standard.md, host-interface.md, ping-protocol.md
config/            csr.example.toml, csr.service (systemd)
docs/              decisions.md, open-questions.md, testing.md, milestones/
```

**CHANGE from README:** the dashboard is no longer a mempool fork. It is embedded in `csr`, has no index of its own, and reads chain data live from the node. Update the README's roadmap accordingly in milestone A1.

---

## 3. Networks

- Config `network = "testnet" | "mainnet"`, default testnet; CLI override `--network`.
- Separate data directories: `<data_root>/testnet/`, `<data_root>/mainnet/`. Keys, storage, DB and earnings never mix.
- Startup check: `get_blockchain_info().chain` must be `ckb_testnet` for testnet, `ckb` for mainnet; otherwise refuse to start and print both.
- First mainnet start requires `--confirm-mainnet` once (stored in the mainnet data dir).
- Dashboard badge: **TESTNET** (amber) / **MAINNET** (red).
- Each network has its own monitor URL and its own deployed script hashes (`config/networks/testnet.toml`, `mainnet.toml`, shipped with the binary).

---

## 4. Node connection and R2 guard

### 4.1 Supported nodes

| Node | Supported |
|---|---|
| `ckb` full node | yes |
| Neuron, full-node mode | yes |
| Neuron light-client mode, `ckb-light-client` | no (refused) |
| Any non-localhost RPC | no (refused at config validation, not overridable) |

- `node.rpc_url` default `http://127.0.0.1:8114`; only `127.0.0.1`, `::1`, `localhost` accepted.
- `csr detect`: probes local ports, reports chain, version, node type, whether the **Indexer** RPC module is enabled. CSR has no index of its own and needs the node's indexer for cell queries. If disabled, CSR tells the operator which `ckb.toml` line to enable; it never edits that file.
- Light-client detection: `sync_state` / `get_blockchain_info` missing → "light client or unsupported node".
- Neuron specifics (RPC port, indexer availability, testnet mode) → verify and record in `docs/open-questions.md`.

### 4.2 R2 guard (not configurable)

Runs every 10 s and before each job start.

| Check | Pass condition |
|---|---|
| Not in initial block download | IBD = false |
| Tip lag vs best known block | ≤ 5 blocks |
| Outbound peers | ≥ 1 |
| Indexer lag vs tip | ≤ 5 blocks |
| Chain | matches configured network |

States: `Synced`, `Syncing`, `Degraded`, `Unreachable`. If not `Synced`: pause running jobs, start no new ones, stop answering pings with "synced" (answers report the real state), show the reason. **No config key can disable this**; a test asserts it.

### 4.3 The node always wins

- Services run at lowest OS priority.
- Hold new jobs if CPU > `limits.pause_above_cpu_percent` for 30 s, free RAM < `limits.min_free_ram_mb`, or free disk < `limits.min_free_disk_gb`.

---

## 5. Identity and keys

| Key | Purpose | Location |
|---|---|---|
| **Service key** (secp256k1) | signs ping answers, job results, float/registry runner-path witnesses | `<net>/keys/service.key`, mode 600 |
| **Storage key** | encrypts service storage | `<net>/keys/storage.key`, mode 600 |

- Generated on first run per network. Never shown or logged; only public keys/hashes are displayed.
- **Node identity** = P2P peer ID from `local_node_info.node_id`.
- **Binding statement:** "service key X belongs to peer ID Y on network N", signed with the service key and included in pings and registration. How to additionally prove it with the node's P2P key without reading the node's data dir → open question.
- **The operator's wallet is never in CSR.** Registration, float creation and withdrawals are signed by the operator in their own wallet (§9).

---

## 6. Ping and ranking (CSR side)

Full protocol in `spec/ping-protocol.md` (write it in milestone A1; the monitor implements the other side).

### 6.1 Modes (setting `ranking.ping_mode`)

| Mode | For | How |
|---|---|---|
| `inbound` (default) | publicly reachable nodes | monitor calls `POST http://<node>:8131/v1/challenge` |
| `outbound` | nodes behind NAT | CSR calls `POST <monitor>/v1/checkin` every 10 min; monitor returns a challenge; CSR posts the answer to `/v1/answer` |
| `both` | reachable nodes that also want check-ins | both of the above |

### 6.2 Challenge and answer

- Challenge: `{ nonce (32 bytes), height, issued_at, monitor_sig }`. CSR verifies `monitor_sig` against the monitor public key in the network config and rejects stale challenges (> 60 s).
- Answer: `{ nonce, peer_id, service_pubkey, binding_sig, network, csr_version, node_version, r2_state, tip_number, tip_hash, block_hash_at_height, uptime_s, ping_mode }`, signed with the service key.
- The ping port serves **only** these endpoints, rate-limited (e.g. 10 req/min per IP), no other data. It binds to `ranking.ping_bind` (default `0.0.0.0:8131`) only when `ranking.join = true` and mode includes `inbound`.
- If `ranking.join = false`: no ping answers, no check-ins, no registration, no jobs. The Network page shows "Join the ranking to see stats".

### 6.3 Network page data

CSR reads the monitor's public API (outbound HTTPS, cached 5 min) for: nodes worldwide, per-country counts (map), own rank, own availability (7/30/365 days), check type (P2P-verified / challenge-only), attestation status.

---

## 7. Dashboard

### 7.1 Principles

- Served by `csr` from embedded files; vanilla JS, inline SVG charts, no framework, no build step.
- Assets < 200 KB; CSR idle RAM < 50 MB.
- No chain index: chain figures come live from node RPC; CSR stores only its own data (jobs, earnings, settings, metric samples) in SQLite.
- Live updates via SSE (`/api/events`), polling fallback. Phone-width layout, light/dark theme.
- Modest cell-model look (cells drawn as boxes with capacity/lock/type/data) where it helps understanding.

### 7.2 Binding and auth

- Default `127.0.0.1:8130`, configurable. Non-loopback bind requires a password (`csr set-password`), else CSR refuses to start.
- Write requests need session + CSRF token, also on localhost.

### 7.3 Pages

| Page | Content |
|---|---|
| **Overview** | network badge, version, uptime; node card (sync state, tip, lag, epoch, peers in/out, node type/version); runtime card (on/off/paused + reason, jobs running/pending); resources card; float card (balance, earned 7/30 days); ranking card (rank, availability); alerts |
| **Node** | details, tip-lag and sync charts (CSR's own samples), tx pool, peers (addresses hidden by default), indexer tip/lag |
| **Network** | nodes worldwide, country map, own rank and availability, check type, attestation status, ping mode and last successful check |
| **Services** | known services: name, code hash, code cell outpoint, language (if declared), status (approved+enabled / approved+disabled / not approved), jobs, earned, storage used; approve/revoke/enable/disable. Discovered services are never auto-approved |
| **Jobs** | tabs: Pending (discovered/queued/scheduled), Active (running/paused/submitting), Closed (settled), Failed (failed/rejected/expired/lost), Long-lived (stored state still held). Row: id, service, kind, trigger, tip, tip rate (CKB per M cycles), cycles used, result tx, status. Detail: timeline, logs (capped), limits applied |
| **Earnings & float** | float balance and history, tips earned, fees paid, net growth per day (chart), settlements table, **Withdraw** and **Top up** buttons (prepare unsigned tx, §9) |
| **Resources** | CPU (total, CSR, ckb process, allocated to services), RAM, disk (node volume, CSR storage vs quota, per service), 24 h charts, headroom (how many more jobs fit) |
| **Settings** | §8 |
| **Logs** | last 1,000 lines, level filter, download. Secrets never logged (tested) |

---

## 8. Configuration and settings

File `<data_root>/csr.toml`; example in `config/csr.example.toml`.

```toml
network = "testnet"

[node]
rpc_url = "http://127.0.0.1:8114"     # localhost only
node_type = "auto"                    # auto | ckb | neuron

[dashboard]
bind = "127.0.0.1:8130"
password_hash = ""

[ranking]
join = true                           # default on; off = no pings, no registration, no jobs
ping_mode = "inbound"                 # inbound | outbound | both
ping_bind = "0.0.0.0:8131"

[runtime]
enabled = false                       # operator turns it on (mainnet: inactive in Phase 1)
allowed_services = []                 # approved code hashes
min_ckb_per_mcycle = 0.0              # minimum tip per million cycles; lower-paid jobs ignored

[limits]
max_concurrent_jobs = 2
max_cycles_per_job = 70_000_000
max_vm_memory_mb = 4
max_job_wall_seconds = 30
pause_above_cpu_percent = 85
min_free_ram_mb = 1024
min_free_disk_gb = 10

[storage]
path = "service_data"                 # under the network data dir, outside the node's data dir
total_quota_mb = 256
per_service_quota_mb = 64

[logging]
level = "info"
```

Float and registry outpoints are not in the config; CSR finds them on-chain by its service key (§9).

**Not configurable by design:** R2 guard, localhost-only RPC, allowlist requirement, encryption of service storage.

Settings page: runtime on/off, pause all, per-service enable/approve/revoke, `min_ckb_per_mcycle`, all limits, storage quotas, join ranking, ping mode, log level (live); dashboard bind/password, network (restart required, marked as such). Saves are validated, atomic, with `csr.toml.bak`.

---

## 9. On-chain: registry, float, operator signing

All scripts in `contracts/` (Rust + ckb-std), deployed on testnet/devnet by `csr-dev`. Exact molecule schemas go in `spec/`.

### 9.1 Operator signing flow (no wallet in CSR)

For registration, float creation, top-up and withdrawal, CSR **prepares an unsigned transaction** and offers:

1. **Export JSON** for offline signing in Neuron and for `ckb-cli` (verify both formats; record findings).
2. **Import signed JSON** and broadcast via the local node.

A browser signing page (JoyID via CCC) can be added later as a vendored, pre-built asset; not in v0.2.

### 9.2 Registry cell (one per node)

- **Type script `csr-registry`**, data: `peer_id, service_pubkey, float_lock_hash, network, attestation, attestation_expiry`, reserved field for CRT prekeys.
- Creation requires a valid **attestation**: the CSR Network Monitor's signature over `(peer_id, service_pubkey, network, expiry)`. The monitor public key is read from a config cell (cell dep) deployed with the scripts.
- **Lock (dual path):** service key may update only `attestation` and the reserved prekey field; the owner lock may update anything or destroy the cell.
- CSR requests an attestation from the monitor after enough successful pings (rule set by the monitor), and refreshes it before expiry.
- CSR refuses to prepare a registration unless R2 is `Synced` (convenience; the attestation is the real check).

### 9.3 Float cell

- **Lock `csr-float`**, args: `owner_lock_hash, service_pubkey_hash, max_fee_shannons`.
- **Runner path:** valid with a service-key signature only if the transaction has exactly one output with the same `csr-float` lock and `output_capacity ≥ input_capacity − max_fee_shannons`. Any other output from the float is rejected.
- **Owner path:** owner lock in the same transaction (any amount, any destination).
- `max_fee_shannons` is chosen by the owner at creation; CSR proposes a default (e.g. 0.01 CKB, to confirm).
- The dashboard shows balance, growth and a warning when the balance is too low to pay the next job fee.

### 9.4 Exploit tests (required)

- service key tries to send float capacity to another lock → rejected
- runner path with fee above `max_fee` → rejected
- registry creation without / with wrong / with expired attestation → rejected
- service key tries to change `float_lock_hash` in registry → rejected

---

## 10. Jobs

### 10.1 Kinds

| Kind | Example | Runners | v0.2 |
|---|---|---|---|
| **Keeper** | finalize a CrowdCell campaign; run a transaction every N blocks | first valid wins | implement |
| **Custodian** | CRT key shares | many, threshold | reserve hooks only (built in the CRT project) |

### 10.2 Job cell standard (keeper)

First task of milestone A3: write `spec/job-cell-standard.md`. Baseline:

- **Type `csr-job`**, args = service code hash.
- **Data:** `trigger, tip_shannons, max_cycles, payload, expiry_block, refund_lock_hash, remaining_runs`.
- **Triggers:** `OnDemand`, `AtBlock(n)`, `AtEpoch(e)`, `AtTimestamp(t)` (block median time), `Every(blocks)` with `remaining_runs`, `OnCellConsumed(outpoint)`.
- **Lock `csr-job-lock`:** spendable when (a) the trigger condition holds (checked via header deps / since), (b) the tip goes to a `csr-float` lock that belongs to a **registered** node (registry cell as cell dep), (c) for `Every`, the job cell is recreated with `remaining_runs − 1` and reduced capacity; or by the `refund_lock_hash` owner after `expiry_block`.
- The tip is set by the **consumer** (the app creating the job).

### 10.3 Execution model

1. **Discovery:** each new block, query the node indexer for `csr-job` cells (prefix search on args = allowlisted code hashes).
2. **Selection:** compute tip rate = `tip_shannons / 1e8 / (max_cycles / 1e6)` CKB per M cycles. Drop jobs below `min_ckb_per_mcycle` or above `max_cycles_per_job`. Order by tip rate, highest first; fill up to `max_concurrent_jobs`.
3. **Run:** load the service code cell, run it in the sandbox with the job payload. The service returns a **transaction skeleton** (inputs/outputs/deps it needs, e.g. CrowdCell finalize).
4. **Complete:** CSR adds the job cell input, float input/output (fee from float), tip output to the float, signs the runner-path witnesses, and submits.
5. **Track:** committed → `Settled` (tip recorded); job cell spent by another runner → `Lost` (no cost, since the transaction never committed).

Job states: `Discovered → Queued/Scheduled → Running ⇄ Paused → Completed → Submitting → Settled | Lost`; side exits `Failed`, `Rejected`, `Expired`. All transitions persisted; on restart `Running` → `Queued`.

### 10.4 Earnings

Earned = tips in `Settled` jobs, verified on-chain. Fees paid tracked separately. Dashboard shows tips, fees and net float growth; never estimates.

---

## 11. Runtime (sandbox)

### 11.1 Execution

- One CKB-VM instance per job, limits from config (cycles, memory, wall time). Nothing shared between jobs except the service's own storage.
- **Languages:** anything compiling to RISC-V. v0.2 supports and tests **Rust (ckb-std), C (ckb-c-stdlib), JavaScript (ckb-js-vm)**. Interpreted services reference an interpreter code cell, as on-chain.

### 11.2 Host interface

Write `spec/host-interface.md` in milestone A2. It is a **superset of the CKB script syscalls** (so existing toolchains work) plus service calls in a separate syscall number range:

| Service call | Purpose |
|---|---|
| `job_payload` | read the job payload |
| `block_info` | tip number, epoch, median time (no wall clock) |
| `chain_read` | read a live cell, header or transaction by hash |
| `cells_query` | indexer query (script + prefix), result-count capped |
| `kv_get` / `kv_put` / `kv_delete` | own storage namespace |
| `kv_mark_secret` | exclude entry from export/backup |
| `delete_on_consume` | delete an entry when an outpoint is consumed (enforced even if the service is disabled) |
| `sign` | host signs a 32-byte message with the service key |
| `tx_skeleton_put` | return the transaction skeleton |
| `result_put` | return result bytes (capped) |
| `debug` | log a line (size/rate capped) |
| *(reserved)* `random_bytes`, `prekey_decrypt` | for CRT; not implemented in v0.2 |

Deliberately absent: network, filesystem, processes, wall clock.

### 11.3 Service storage

- Per service code hash under `<net>/service_data/<code_hash>/`, outside the node data dir.
- Encrypted (ChaCha20-Poly1305) with the storage key; atomic writes; per-service and total quotas (over quota → error to service).
- `delete_on_consume` rules checked every block; deletions logged (time, block, outpoint, never content).

---

## 12. HTTP API (dashboard)

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/status` | network, version, uptime, R2 state, runtime state, phase gate |
| GET | `/api/node` | node details |
| GET | `/api/network` | ranking data (cached from monitor) |
| GET | `/api/resources`, `/api/resources/history` | current + series |
| GET | `/api/services`; POST `/api/services/{hash}/{approve,revoke,enable,disable}` | services |
| GET | `/api/jobs`, `/api/jobs/{id}` | jobs |
| GET | `/api/earnings` | tips, fees, float history, settlements |
| POST | `/api/float/prepare-{create,topup,withdraw}`, `/api/registry/prepare-create`, `/api/tx/submit-signed` | operator signing flow |
| GET/PUT | `/api/settings` | read/save settings |
| POST | `/api/runtime/{pause,resume}` | pause/resume |
| GET | `/api/events` | SSE |
| GET | `/api/logs` | logs |
| GET | `/healthz` | health |

Ping endpoints live on the separate ping port (§6).

---

## 13. CLI

```
csr run [--network testnet|mainnet] [--config PATH] [--confirm-mainnet]
csr detect | check | set-password | version
csr service list|approve|revoke <code_hash>
csr jobs list [--state ...]
csr float status | prepare-create | prepare-topup <CKB> | prepare-withdraw <CKB>
csr register prepare | status
csr tx submit <signed.json>
csr export-state            # jobs/earnings JSON; never secret storage
```

`csr-dev` (refuses to run against mainnet; key from env `CSR_DEV_TESTNET_KEY`):

```
csr-dev deploy-contracts          # registry, float, job scripts + monitor config cell
csr-dev deploy-service <elf>      # code cell for a service
csr-dev job <code_hash> --tip <CKB> --trigger ondemand|at-block:N|every:N --runs K --payload <hex>
csr-dev attest <peer_id> <pubkey> # local mock attestation for devnet
```

Targets: Linux x86_64 first (systemd unit in `config/csr.service`), Windows x86_64 second, macOS best effort.

---

## 14. Milestones

| ID | Scope | Done when |
|---|---|---|
| **A1** | Workspace, config, networks, node RPC client, `csr detect/check`, R2 guard, resource monitor, SQLite state, dashboard (Overview, Node, Resources, Settings, Logs), auth, SSE, **ping responder (inbound + outbound) against the mock monitor**, `spec/ping-protocol.md`, README roadmap update | runs a week beside a testnet `ckb` node and Neuron; R2 failure tests pass; ping answers verified by the mock; assets < 200 KB |
| **A1+** | Network page + monitor API client (mock until the real monitor is live) | figures shown match the monitor/mock |
| **A2** | CKB-VM sandbox, host interface v0 (local), encrypted storage, local job execution from an ELF file, `hello` service in Rust, C and JS, Services page (local) | identical results in all three languages; cycle/memory/time limit tests; storage survives restart |
| **A3** | `spec/job-cell-standard.md`; contracts (registry, float lock, job type/lock) + exploit tests; `csr-dev`; operator signing flow; on-chain discovery, selection by tip rate, scheduler, settlement; Jobs and Earnings & float pages; demo services `heartbeat` and `crowdcell-finalize` | devnet then testnet: registration only with attestation; float cannot be drained; float grows by tip − fee; two runners racing — loser pays nothing; heartbeat runs unattended for days; a CrowdCell test campaign finalized by CSR |
| **A4** | Mainnet gate, packaging (release build, systemd unit), install docs for operators | mainnet start shows dashboard + ranking with runtime greyed; release binary < 30 MB |

A1, A1+ and A4's packaging are what operators install in Phase 1. A2 and A3 continue on testnet during Phase 1.

---

## 15. Testing

- **Unit:** config validation (localhost-only, no R2 override), R2 state machine, job state machine, tip-rate selection, quotas, encryption round-trip, ping signature verification and stale-challenge rejection, secrets never logged.
- **Integration (no node):** mocked node RPC with recorded testnet fixtures; mock monitor.
- **VM:** example services in-process; limit violations; unknown syscall; oversized result.
- **Contracts:** ckb-testtool unit tests incl. all exploit tests in §9.4.
- **Devnet harness:** OffCKB + N CSR instances + mock monitor; debug-only option for several CSR instances sharing one devnet node.
- **Testnet E2E (manual, `docs/testing.md`):** registration, float, heartbeat, CrowdCell finalize.
- **Never** a mainnet transaction in any automated test.

---

## 16. Non-functional targets

| Item | Target |
|---|---|
| Idle RAM / CPU | < 50 MB / < 1 % of one core |
| Dashboard assets | < 200 KB, no CDN |
| Startup to dashboard | < 3 s |
| RPC load with runtime off | ≤ 1 request/s average |
| Ping port | ping endpoints only, rate-limited |
| Files written | only under CSR's data root |

---

## 17. Open questions

1. Neuron: RPC port, indexer availability, testnet full-node vs light-client mode, offline-signing JSON format.
2. Proving the peer-ID binding with the node's P2P key without reading the node's data dir.
3. Default `max_fee_shannons` for the float.
4. Attestation rule (how many successful pings, over how long) and expiry — defined in the monitor spec.
5. Exact syscall number range for service calls.
6. Phase 2 unlock source: stat cell field read by `mainnet_runtime_unlocked()`.
