# CSR host interface (A2)

Services are RISC-V programs run in CKB-VM (`ckb-vm` 0.24.15, ISA `IMC|B|MOP`, VM version 2 as for
`hash_type = data2`). The host answers the standard CKB syscalls from a **virtual transaction** and
adds the **service calls 7001–7099**. Syscall convention: number in `a7`, arguments in `a0..a5`,
return value in `a0`. Every syscall costs 500 cycles.

## Virtual transaction

* one input, the virtual job cell, whose **type script** is the service script: `code_hash` = data
  hash of the program run in the VM, `hash_type = data2`, `args` from the manifest. The service
  runs as the script group of input 0;
* cell deps: the program (index 0), then every manifest `data_cells` file;
* `witnesses[0]` = the raw job payload.

## CKB syscalls answered

`vm_version` (2041, returns 2), `current_cycles` (2042), `load_script` (2052), `load_tx_hash` (2061),
`load_script_hash` (2062), `load_cell` (2071), `load_header` (2072, always index-out-of-bound), `load_input` (2073),
`load_witness` (2074), `load_cell_by_field` (2081), `load_header_by_field` (2082), `load_input_by_field` (2083),
`load_cell_data_as_code` (2091), `load_cell_data` (2092), `load_block_extension` (2104), `debug` (2177),
`exit` (93, by the VM). Not provided (`unknown_syscall`): `load_transaction`, `exec`, `spawn` and the process/pipe family.

## Return codes of service calls

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | item missing |
| 2 | not supported (reserved / local mode) |
| 10 | quota exceeded |
| 11 | too large |
| 12 | invalid argument |

Buffer-out calls follow CKB's partial loading: `len_ptr` points to a u64 holding the buffer size on
entry and the full data length on return; at most `size` bytes starting at `offset` are written.

## Calls

| No. | Call | Behaviour |
|---|---|---|
| 7001 | `job_payload(addr, len_ptr, offset)` | the payload (same bytes as witness 0) |
| 7002 | `block_info(addr)` | 24 bytes LE: tip number, packed epoch, median time (ms); local runs: `--block-info` or zeros |
| 7003 | `chain_read(kind, key, addr, len_ptr, offset)` | reads from the node (job runs only; 2 in local mode), see below |
| 7004 | `cells_query(script, script_len, script_type, addr, len_ptr, offset)` | indexer query (job runs only; 2 in local mode), see below |
| 7010 | `kv_get(key, key_len, addr, len_ptr, offset)` | 0, or 1 missing; 12 for an empty key or one over 256 B |
| 7011 | `kv_put(key, key_len, val, val_len)` | 0; 10 quota exceeded; 11 key > 256 B or value > 1 MiB; 12 empty key |
| 7012 | `kv_delete(key, key_len)` | 0 / 1 missing |
| 7013 | `kv_mark_secret(key, key_len)` | entry is excluded from any export/backup; 0 / 1 missing |
| 7014 | `delete_on_consume(key, key_len, outpoint)` | `outpoint` = 36 bytes (tx hash ‖ index u32 LE); the entry is deleted when that outpoint is consumed, even if the service is disabled. The key need not exist yet |
| 7020 | `sign(msg32, sig_out64)` | see below |
| 7030 | `tx_skeleton_put(addr, len)` | stores the transaction skeleton (see below); the last call wins |
| 7031 | `result_put(addr, len)` | result bytes; more than **65,536** bytes stops the run with `result_too_large` |
| 7050, 7051 | `random_bytes`, `prekey_decrypt` | reserved, return 2 |

Other numbers in 7001–7099 return 2. Numbers outside CKB's set and 7001–7099 stop the run with
status `unknown_syscall`. The CKB `debug` syscall output is collected (max 64 lines, 1 KB each).

## Chain calls (A3c)

Available when CSR runs an on-chain job (`csr run`); `csr service run-local` returns 2.

* `chain_read(kind, key, addr, len_ptr, offset)` – `kind` 0: live cell output (molecule `CellOutput`), 1: live
  cell data, both with `key` = 36-byte out point (tx hash ‖ index u32 LE); 2: header (molecule `Header`), 3:
  transaction (molecule `Transaction`), both with `key` = 32-byte hash. Returns 0, 1 (dead / unknown) or 12
  (unknown kind). Partial loading as above.
* `cells_query(script, script_len, script_type, addr, len_ptr, offset)` – `script` is a molecule `Script`
  (at most 4096 bytes), `script_type` 0 = lock, 1 = type; the args are matched as a **prefix**. Writes a
  molecule `OutPointVec` (u32 LE count, then 36-byte out points) of at most **64** live cells, in indexer
  order. Returns 0, or 12 for a malformed script / script type.

A failing node request stops the run (`script_error`). The VM time spent waiting for the node counts
towards the wall-time limit.

## Transaction skeleton (A3c)

`tx_skeleton_put` takes a molecule `Transaction` (CKB's own, as built by ckb-types / ckb-std). Empty (or
never called) = the job needs no extra cells. CSR appends the skeleton's inputs (with their `since`), cell
deps, header deps, outputs with data and witnesses after its own: the run transaction is inputs
`[runner wallet cell, job cell, skeleton inputs…]`, outputs `[runner wallet cell + tip, recreated
job | refund, skeleton outputs…]`, witnesses `[runner wallet signature, empty, skeleton witnesses…]`.
The skeleton must bring the capacity of its outputs (Σ outputs ≤ Σ of its live inputs; any surplus goes to
the miner). It may not contain a job cell (the job lock allows exactly one).

## Result

The last `result_put` wins. If a service never calls it, the host uses the last debug line of the
form `CSR-RESULT:0x<hex>`; else the result is empty. This is how JavaScript services on ckb-js-vm
(which cannot issue raw syscalls) return results; a debug line is cut at 1 KB, so the fallback
carries at most 505 bytes.

## Signing (7020)

The host signs with the service key (`keys/service.key`): ECDSA secp256k1 over the SHA-256 digest of

    "CSR-SERVICE-SIGN-v1\n" ‖ code_hash (32 B) ‖ msg32

and writes the 64-byte compact (r ‖ s) low-S signature. The prefix and the code hash make sure that
a service can never get the service key to sign a transaction, a ping answer or another service's
message.

## Storage

Per service under `<data-root>/<network>/service_data/<code_hash without 0x>/`. Each entry is a
file named by `HMAC-SHA256(storage.key, "csr-storage-name-v1" ‖ key)` holding
`nonce(12) ‖ ChaCha20-Poly1305(secret flag ‖ key_len u16 LE ‖ key ‖ value)` (random nonce per
write, file name as associated data, atomic temp-file + rename). Quotas
(`storage.per_service_quota_mb`, `storage.total_quota_mb`) count plaintext key + value bytes.
`delete_on_consume` rules are kept encrypted in `rules.bin`; `csr storage simulate-consumed
--outpoint 0x<64 hex>:<index>` applies them and appends `time service outpoint` (never content) to
`service_data/deletions.log`.

## Limits

Cycles (`limits.max_cycles_per_job`, `--max-cycles`), memory (fixed 4 MiB in the pinned ckb-vm), wall
time (`limits.max_job_wall_seconds`, enforced through ckb-vm's pause flag). No network, filesystem,
process or wall-clock access for services.
