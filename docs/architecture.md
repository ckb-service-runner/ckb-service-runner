# Architecture (draft)

## Overview

```
┌──────────────────────────── operator's machine ────────────────────────────┐
│                                                                             │
│  ┌──────────────┐   local RPC    ┌───────────────────────────────────────┐ │
│  │  CKB full    │◄──────────────►│  ckb-service-runner                   │ │
│  │  node (ckb)  │  127.0.0.1     │   • node checks (R2)                  │ │
│  │  unchanged   │                │   • ranking client (R1)               │ │
│  └──────┬───────┘                │   • runtime: CKB-VM sandbox (R3)      │ │
│         │ P2P                    │   • service storage (R4)              │ │
│         │                        │   • dashboard + settings (web UI)     │ │
└─────────┼────────────────────────┴───────────────────────────────────────┴─┘
          │
          ▼
   independent probes ──► weekly signed summary ──► stat cell on CKB
```

The runner is a **separate process and a separate install**. It never modifies
the `ckb` binary, its config or its data directory. A crash or exploit in a
service cannot take the node down, and the runner has its own release schedule.

## R1 — Node reputation ledger

Recorded per node, keyed by P2P peer ID (not IP):

- availability (% of probes answered, 7 / 30 / 365 days)
- sync lag behind chain tip
- software version, first-seen date
- region, country, hosting provider (ASN)
- later: service results (answered correctly and on time)

Design choices:

- **Several independent probes**, each signing its results. Phase 1 starts with
  one project-run probe; decentralising it is a Phase 2 goal.
- **Weekly summary on-chain**, details off-chain: the stat cell holds a summary
  and a hash of the full data set.
- **Only runner nodes** are ranked. Joining is a setting (default on) and is
  required to be assigned jobs.
- It is reputation, not proof — documented as such.

## R2 — Fully synced full node

Local check, in code, not configurable:

- node RPC must be localhost; remote URLs are refused
- before start and before every job: not in initial block download, tip within
  a few blocks of the best known tip, has outbound peers
- if a check fails: pause running jobs, start no new ones

Remote proof:

- the runner signs "service key X belongs to peer ID Y" with the node's P2P key
- probes request headers and blocks at random heights **over P2P**, plus tip
  challenges with a time limit
- failures lower the node's reputation (R1)

Residual gap: nobody can prove from outside that a node verified scripts.

## R3 — Secure execution

Services run in CKB-VM and only see a small set of host calls:

| Host call | Purpose |
|---|---|
| `chain_read` | read chain state from the local node |
| `request_recv` | receive a job |
| `sign` | ask the host to sign with the service key (service never sees keys) |
| `kv_get` / `kv_put` / `kv_delete` | own storage namespace (R4) |
| `delete_on_consume` | register deletion when a given cell is consumed |

Deliberately absent: outbound network, file system, processes, clock beyond
block time. Cycle, memory, concurrency and storage limits come from config.

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
- event-driven deletion runs even when the service is stopped; deletions are
  logged (time and cell, never content)
- entries marked secret are excluded from backups and exports

Known limit: deletion is enforced by honest software, not by maths. Mitigations
are node diversity, bonding/slashing on leaks, and thresholds.

## Operator safety rules

- **Allowlist, not auto-discovery.** Only code hashes the operator approved run.
- **The runner wallet, no operator wallet signing.** The runner owns a plain wallet (cells of its own service-key lock) that pays the registration and its fees and receives the tips; it sweeps surplus to the operator's `sweep_address` (any full-format address). The DAO stake stays in the operator's wallet, which consents once by signing a text.
- **The node always wins.** Services pause while syncing or under load.
