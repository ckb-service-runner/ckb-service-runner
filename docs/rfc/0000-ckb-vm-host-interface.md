---
Number: "0000"
Category: Standards Track
Status: Draft
Author: Patrick Sawyer
Created: 2026-11-10
---

# CKB-VM host interface for off-chain keeper services

## Abstract

This RFC proposes a shared range of CKB-VM syscall numbers, 7001–7099, for **off-chain keeper services**: RISC-V programs that a full-node operator ("runner") executes in CKB-VM to build and sign follow-up transactions for job cells. The range gives services a stable way to read their job payload, query the node, keep private state, sign with a service key, and return a transaction skeleton and a result. It changes nothing on chain. Its purpose is that a service compiled once runs unchanged on every conforming runner.

The normative interface is `spec/host-interface.md` of the reference implementation (CKB Service Runner, "CSR"). If this draft and that spec differ, the spec wins.

## Motivation

Many CKB applications need a keeper: someone has to notice that a condition holds (a timelock elapsed, an auction closed, a cell became claimable) and send a transaction. Today each application ships its own bot with its own node access, key handling and state files. Full-node operators could run such keepers for many applications, but only if the service is a portable artifact rather than a custom daemon.

CKB-VM is already the right execution environment: it is deterministic, metered in cycles, sandboxed, and its toolchains (C, Rust through ckb-std, JavaScript on ckb-js-vm) already exist. A service is an ordinary CKB script binary, identified by its data hash and run with `hash_type = data2` semantics. What is missing is the host side: the standard CKB syscalls only describe a transaction being verified, not a program that reads chain state and produces a transaction.

If every runner invented its own calls, a service would be tied to one runner. A shared number range means:

* a service author targets one interface and tests it locally (the interface also works without a node, see below);
* a runner can be replaced or run in parallel with others without changing any service;
* tooling (compilers, simulators, linters) can recognise the calls.

## Specification

### Execution model

A service is a RISC-V program run in CKB-VM (`ckb-vm` 0.24.15, ISA `IMC|B|MOP`, VM version 2, as for `hash_type = data2`). Calling convention is CKB's: syscall number in `a7`, arguments in `a0..a5`, return value in `a0`.

The runner builds a **virtual transaction**: one input, the virtual job cell, whose type script is the service script (`code_hash` = data hash of the program, `hash_type = data2`, `args` from the manifest); cell deps are the program (index 0) followed by the manifest's data cells; `witnesses[0]` is the raw job payload. The service runs as the script group of input 0. The standard CKB syscalls are answered from this transaction: 2041 (`vm_version`, returns 2), 2042, 2052, 2061, 2062, 2071, 2072 (always index-out-of-bound), 2073, 2074, 2081, 2082, 2083, 2091, 2092, 2104, 2177 (`debug`) and 93 (`exit`). `load_transaction`, `exec`, `spawn` and the process/pipe family are not provided.

### Return codes

Service calls (7001–7099) return in `a0`:

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | item missing |
| 2 | not supported (reserved, or unavailable in local mode) |
| 10 | quota exceeded |
| 11 | too large |
| 12 | invalid argument |

Calls that write a buffer follow CKB's partial loading: `len_ptr` points to a u64 holding the buffer size on entry and the full data length on return; at most `size` bytes starting at `offset` are written.

### Calls

| No. | Call | Behaviour |
|---|---|---|
| 7001 | `job_payload(addr, len_ptr, offset)` | the payload (same bytes as witness 0) |
| 7002 | `block_info(addr)` | 24 bytes LE: tip number, packed epoch, median time (ms); local runs use `--block-info` or zeros |
| 7003 | `chain_read(kind, key, addr, len_ptr, offset)` | reads from the node; job runs only, 2 in local mode |
| 7004 | `cells_query(script, script_len, script_type, addr, len_ptr, offset)` | indexer query; job runs only, 2 in local mode |
| 7010 | `kv_get(key, key_len, addr, len_ptr, offset)` | 0, or 1 missing; 12 for an empty key or one over 256 B |
| 7011 | `kv_put(key, key_len, val, val_len)` | 0; 10 quota exceeded; 11 key > 256 B or value > 1 MiB; 12 empty key |
| 7012 | `kv_delete(key, key_len)` | 0 / 1 missing |
| 7013 | `kv_mark_secret(key, key_len)` | entry is excluded from any export or backup; 0 / 1 missing |
| 7014 | `delete_on_consume(key, key_len, outpoint)` | `outpoint` is 36 bytes (tx hash ‖ index u32 LE); the entry is deleted when that out point is consumed, even if the service is disabled; the key need not exist yet |
| 7020 | `sign(msg32, sig_out64)` | see below |
| 7030 | `tx_skeleton_put(addr, len)` | stores the transaction skeleton; the last call wins |
| 7031 | `result_put(addr, len)` | result bytes; more than 65,536 bytes stops the run with `result_too_large` |

**`chain_read`.** `kind` 0: live cell output (molecule `CellOutput`); 1: live cell data; both with `key` = 36-byte out point. `kind` 2: header (molecule `Header`); 3: transaction (molecule `Transaction`); both with `key` = 32-byte hash. Returns 0, 1 (dead or unknown) or 12 (unknown kind).

**`cells_query`.** `script` is a molecule `Script` of at most 4096 bytes; `script_type` 0 = lock, 1 = type; the args are matched as a prefix. Writes a molecule `OutPointVec` (u32 LE count, then 36-byte out points) of at most 64 live cells, in indexer order. Returns 0, or 12 for a malformed script or script type.

A failing node request stops the run (`script_error`). Time spent waiting for the node counts towards the wall-time limit.

**`sign`.** The host signs with the service key: ECDSA secp256k1 over the SHA-256 digest of `"CSR-SERVICE-SIGN-v1\n" ‖ code_hash (32 B) ‖ msg32` and writes a 64-byte compact (r ‖ s) low-S signature. The prefix and code hash mean a service can never obtain a signature valid for a transaction, another service's message, or any other protocol.

**`tx_skeleton_put`.** Takes a molecule `Transaction` (CKB's own). Empty or never called means the job needs no extra cells. The runner appends the skeleton's inputs (with `since`), cell deps, header deps, outputs with data, and witnesses after its own. The skeleton must bring the capacity of its outputs (sum of outputs ≤ sum of its live inputs; any surplus goes to the miner) and may not contain a job cell.

**Result.** The last `result_put` wins. If a service never calls it, the host uses the last debug line of the form `CSR-RESULT:0x<hex>`, otherwise the result is empty. This lets services on ckb-js-vm, which cannot issue raw syscalls, return results; because a debug line is cut at 1 KB, the fallback carries at most 505 bytes.

### Reserved numbers

7050 (`random_bytes`) and 7051 (`prekey_decrypt`) are reserved and return 2. Every other number in 7001–7099 not listed above also returns 2, so a service can probe for support and later versions can add calls without breaking old services. A service must not rely on any reserved number.

### Cycle cost

Every syscall costs 500 cycles, plus 1 cycle per 4 bytes moved between the VM and the host (rounded up), plus 100,000 cycles per node call (`chain_read`, `cells_query`). The extra charge counts against the cycle limit like VM instructions.

### Limits

Keys: 1–256 bytes. Values: at most 1 MiB. `result_put`: at most 65,536 bytes. `cells_query` script: at most 4096 bytes; answer: at most 64 cells. `debug`: at most 64 lines of 1 KB each, scanning at most 1024 bytes for the terminator. Memory is the fixed 4 MiB of the pinned ckb-vm. Cycles and wall time are set by runner configuration. Storage is subject to per-service and total quotas (code 10).

### What a runner must refuse

* Syscall numbers that are neither CKB's set listed above nor in 7001–7099 stop the run with status `unknown_syscall`.
* Network, file system, process and wall-clock access are not offered to services in any form.
* A skeleton input that is a runner-owned cell (the runner wallet lock, a dual lock naming the runner lock hash, a job cell, or a registry cell) is rejected before signing and the run fails.
* A skeleton containing a job cell is rejected.
* A `sign` request is never signed over a bare digest; the domain prefix is mandatory.
* Results over 65,536 bytes stop the run.

## Service identity

* **Service ID.** The 32-byte value a job cell names. For a single binary without args or data cells it is the program's data hash (`code_hash`). Otherwise it is `blake2b_256("CSR-SERVICE-v1" ‖ code_hash ‖ u32 LE len(args) ‖ args ‖ u8 n ‖ data_hash_1 … data_hash_n)` with CKB's `ckb-default-hash` personalization.
* **Manifest cell.** A cell whose type script (`csr-service-manifest`, `hash_type = data2`, `args` = Type ID) binds a molecule `ServiceManifestV1` to a code cell: `code_hash`, the code cell out point, 0–4 data cells in cell-dep order, script `args` (0–64 bytes), and descriptive text. The type script requires that the manifest's lock is the lock of the code cell, so only the publisher of the code can publish its manifest. A manifest is information only; a runner executes nothing until its admin enables the service.
* **`data2` references.** The program and its data cells are referenced by data hash and executed under `hash_type = data2` semantics, so a service is immutable: a new binary is a new manifest and a new service ID.

## Security considerations

* **Sandbox.** Services run in CKB-VM with fixed memory and a cycle and wall-time budget. They get no network, file system, process or wall-clock access; the only I/O is the calls above.
* **Skeleton guard.** A service does not sign transactions. It proposes a skeleton; the runner assembles the transaction, adds its own wallet input and signature, and rejects skeletons that spend runner-owned cells. A buggy or malicious service therefore cannot spend the runner's funds beyond the fee and tip the runner already put in.
* **Service key isolation.** `sign` is domain-separated by prefix and code hash, so a service cannot obtain a signature usable on chain or by another service.
* **Storage encryption.** Per-service storage is keyed by a service key derived (HMAC-SHA256) from a storage key and the code hash; each entry is sealed with ChaCha20-Poly1305 with a random nonce and AAD binding the code hash and file name, so an entry cannot be moved into another service and file names do not reveal keys. `kv_mark_secret` excludes entries from export; `delete_on_consume` lets a service ensure secrets are erased once the cell they protect is spent.
* **Trust-minimised model and its limits.** Execution is deterministic and the program is content-addressed, so anyone can re-run a service on the same inputs. But the runner is trusted for liveness and for the honesty of its node: `chain_read` and `cells_query` answer from the runner's own node, and a runner may refuse or delay a job. A service's output is not proven on chain; the safety of a job comes from the on-chain job-cell rules, which constrain what the resulting transaction may do. This RFC offers no defence against a dishonest runner censoring or front-running its own jobs.
* **Resource exhaustion.** Cycle accounting charges for data moved and for node calls, bounding the work a service can force.

## Compatibility

* **No consensus change.** The interface exists only between a service and an off-chain runner. Nothing is verified by nodes, and no script, transaction format or hardfork is involved.
* **No collision with CKB.** CKB's own syscalls use 93 and the 2041–2177 range; 7001–7099 does not overlap any of them.
* **Local mode.** `chain_read` and `cells_query` return 2 when run without a node, so services can be tested offline.
* **Future CKB-VM.** A future CKB-VM or ckb-std could reserve 7001–7099 in its documentation as "host-defined service calls, not available in on-chain script execution", so that on-chain VMs keep treating them as unknown syscalls and tools can share the names. This needs no VM change, only a registry entry.
* **Versioning.** New calls take unused numbers in the range; unlisted numbers already return 2, so old services keep working.

## Reference implementation

CKB Service Runner (this repository): the host in the runner, the normative `spec/host-interface.md`, `spec/job-cell-standard.md` and `spec/service-manifest.md`, the on-chain job-cell and manifest contracts, example services and a JavaScript job builder. `docs/architecture.md` and `docs/decisions.md` describe the design.

## Open questions

1. Should the range 7001–7099 be registered in an RFC or in ckb-std/ckb-vm documentation, and who owns the registry?
2. Is a shared range preferable to a single `spawn`-style or `exec`-style extension, now that CKB has process syscalls? The reference host does not provide them.
3. Should the cycle price (500 per call, 1 per 4 bytes, 100,000 per node call) be normative, or left to runners?
4. Should `chain_read` and `cells_query` be pinned to a block, to make runs reproducible by third parties?
5. Reserved calls (7050, 7051) are placeholders for randomness and prekey decryption; should they be specified here or left out?
6. Is domain-separated signing with a per-service key sufficient, or should a standard key-derivation scheme be specified?
