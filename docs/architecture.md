# Architecture

## Overview

```
┌──────────────────────────── operator's machine ────────────────────────────┐
│                                                                             │
│  ┌──────────────┐   local RPC    ┌───────────────────────────────────────┐ │
│  │  CKB full    │◄──────────────►│  csr (CKB Service Runner)             │ │
│  │  node (ckb)  │  127.0.0.1     │   • node checks (R2)                  │ │
│  │  unchanged   │                │   • ranking client / ping responder   │ │
│  └──────┬───────┘                │   • runtime: CKB-VM sandbox (R3)      │ │
│         │ P2P                    │   • service storage (R4)              │ │
│         │                        │   • runner wallet, registration       │ │
│         │                        │   • dashboard + settings (web UI)     │ │
└─────────┼────────────────────────┴───────────────────────────────────────┴─┘
          │
          ▼
   CSR Network Monitor ──► ranking, daily full-node check, config cell (clock) on CKB
```

The runner is a **separate process and a separate install**. It never modifies
the `ckb` binary, its config or its data directory (the node's P2P key file is only read, and only when the
operator sets `node.p2p_key_file`). A crash or exploit in a
service cannot take the node down, and the runner has its own release schedule.

## Binding and node identity

Two keys identify a runner:

- the **service key** (`keys/service.key`, secp256k1) signs ping answers, the binding, attestation requests and
  every transaction the runner sends; services never see it (they can only use `sign`, which is domain-separated);
- the node's **P2P key** (the `ckb` node's own secret key) proves that the service key belongs to a particular peer ID.

The *binding* says "service key X belongs to peer ID Y". It is signed by the service key **and** by the node's P2P key
(decision D6 A), so the monitor verifies the peer-ID ownership cryptographically. The monitor then sets
`check_type` for the node:

| `check_type` | Meaning |
|---|---|
| `peer_connected` | the node is a peer of the monitor's node |
| `p2p_verified` | the node passed the daily full-node check; its identity is proven |
| `none` | not checked |

The attestation (signed by the monitor, bound to peer ID, service key and network) is what the registry contract
checks when the runner registers. Protocol: `spec/ping-protocol.md`, `spec/registry.md`.

## R1 — Node ranking

The CSR Network Monitor keeps, per node and keyed by peer ID: availability (7 / 30 / 365 days), `check_type`,
attestation, stake status and rank. The runner only reads the public API (`spec/monitor-public-api.md`, cached 5 min)
and shows it on the Network page. The runner does **not** read the stat cell. Only runner nodes that joined
(`ranking.join`) are ranked; it is reputation, not proof.

## R2 — Fully synced full node

Local check, in code, not configurable:

- the node RPC must be loopback; remote URLs are refused;
- the check (not in initial block download, tip within a few blocks of the best known tip, outbound peers, indexer
  available) is read **once per block**, not once per job;
- if the check fails, the runner starts no new run. A run that has started is not interrupted; pauses happen only
  **before a run starts** (decision D4 A).

Remote proof is the monitor's daily full-node check over P2P (above). Residual gap: nobody can prove from outside
that a node verified scripts.

Proof of the node identity:

- binding of service key and node (D6 A): the runner reads the node's P2P secret key (read only, from the
  file the operator grants access to, `node.p2p_key_file`), checks that it derives the node ID the node
  reports, and sends the monitors a node-identity proof — `CSR-NODE-IDENTITY-v1` signed by the P2P key
  **and** the service key (`spec/ping-protocol.md` §5b). The binding signature in the ping answer is signed by
  the service key alone and is only a claim;
- runners behind NAT (D12 A): each monitor publishes the P2P address of its own CKB node; the runner asks its
  node to connect to it (`add_node`, Net RPC module), so the monitor checks the node among its own peers.

## R3 — Secure execution

Services are RISC-V programs in CKB-VM. They answer the standard CKB syscalls from a virtual transaction and use the
service calls 7001–7099 of `spec/host-interface.md`:

| Host call | Purpose |
|---|---|
| `job_payload`, `block_info` | the job's payload, tip number / epoch / time |
| `chain_read`, `cells_query` | read cells, headers, transactions and indexer results from the local node (job runs only) |
| `kv_get` / `kv_put` / `kv_delete` / `kv_mark_secret` | own storage namespace (R4) |
| `delete_on_consume` | register deletion of an entry when a given outpoint is consumed |
| `sign` | sign with the service key, domain-separated by the code hash |
| `tx_skeleton_put`, `result_put` | the extra cells of the run transaction, and the result |

There is no call to receive jobs: the runner discovers job cells, runs the service and hands it the payload.
Deliberately absent: outbound network, file system, processes, wall clock. Cycle, memory, wall-time and storage
limits come from the config.

Job trigger `OnCellConsumed`: not a chain trigger. It is an `OnDemand` job whose payload starts with the
`CSRW` watch header; the keeper checks the watched cells with `get_live_cell` on every new block and selects
the job only after the cell was seen consumed (`spec/job-cell-standard.md`, section 2).

This interface is the main candidate for an RFC.

## R4 — Persistent service storage

- namespace per service code hash (and, for CRT, per switch cell)
- directory outside the node's data dir; survives restarts and resyncs
- atomic writes; encrypted at rest with a runner-held key
- layout (storage format version 2): `<root>/<code_hash hex>/<HMAC(service key, name tag ‖ key)>.e`;
  the service key is derived from the runner key and the code hash, and the service is part of the
  AAD, so an entry cannot be moved to another service; version 1 data is migrated at `csr run` start
- `delete_on_consume` rules live in `rules.bin` (max 256 per service) and count in the quota
- per-service and total quotas; over quota returns an error
- event-driven deletion runs even when the service is stopped: `csr run` checks the out points of
  all `delete_on_consume` rules with `get_live_cell` on every new block and deletes the entries of
  consumed cells, for every service including disabled ones; deletions are logged (time and cell,
  never content). `csr storage simulate-consumed` stays as a manual tool
- entries marked secret are excluded from backups and exports

Known limit: deletion is enforced by honest software, not by maths.

## Jobs, tips and the runner wallet

With `runtime.enabled`, `csr run` discovers job cells (`csr-job-lock`/`csr-job-type`), runs approved on-chain
services and sends the run transaction: inputs `[runner wallet cell, job cell, skeleton inputs…]`. The tip goes to
the runner wallet; the fee comes from the job cell. The **runner wallet** is the set of plain cells of the service
key's lock. It pays the registration and the fees and receives the tips; surplus is swept to `sweep_address`.

## Registration

A registration cell (`csr-registry` type, `csr-dual-lock`) is owned by the runner and references a **stake**: a
Nervos DAO deposit of at least 10,000 CKB that stays in the operator's wallet. The stake owner consents once by
signing a text. The runner signs and sends all of its own transactions: registration, update, removal, settlement,
sweep and `wallet send`. It never signs for the operator.

## Operator safety rules

- **Allowlist, not auto-discovery.** Only code hashes the operator approved run.
- **The runner wallet, no operator wallet signing.** The runner owns a plain wallet (cells of its own service-key lock) that pays the registration and its fees and receives the tips; it sweeps surplus to the operator's `sweep_address` (any full-format address). The DAO stake stays in the operator's wallet, which consents once by signing a text.
- **The node always wins.** Runs that have not started wait while the node syncs or is under load.
