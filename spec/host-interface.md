# Host interface (A2)

Services are RISC-V programs run by CKB-VM (`ckb-vm` 0.24.15, ISA `IMC|B|MOP`, VM version 2 = `hash_type data2`).
They see a **virtual transaction**: input 0 is the virtual job cell whose type script is the service script
(`code_hash` = data hash of the program, `hash_type = data2`, `args` from the manifest); the cell deps are the program
followed by the manifest's `data_cells`; `witnesses[0]` is the raw job payload.

## CKB syscalls answered

`vm_version` (2041, returns 2), `current_cycles` (2042), `load_script` (2052), `load_tx_hash` (2061),
`load_script_hash` (2062), `load_cell` (2071), `load_header` (2072, always index-out-of-bound), `load_input` (2073),
`load_witness` (2074), `load_cell_by_field` (2081), `load_header_by_field` (2082), `load_input_by_field` (2083),
`load_cell_data_as_code` (2091), `load_cell_data` (2092), `load_block_extension` (2104), `debug` (2177),
`exit` (93, by the VM). Every syscall costs 500 cycles. Not provided (stop the run with `unknown_syscall`):
`load_transaction`, `exec`, `spawn` and the process/pipe family.

Debug output is collected: at most 64 lines, each cut at 1024 bytes.

## Service calls (7001–7099)

Return value in `a0`: 0 success, 1 item missing, 2 not supported, 10 quota exceeded, 11 too large, 12 invalid argument.
Buffer-out calls use CKB's partial-loading convention: `len_ptr` holds the buffer size on entry and the full length on return.

| No. | Call | State |
|---|---|---|
| 7001 | `job_payload(addr, len_ptr, offset)` | implemented |
| 7002 | `block_info(addr)` – 24 bytes LE: tip, packed epoch, median time ms | implemented (zeros unless `--block-info`) |
| 7003 | `chain_read` | returns 2 (A3) |
| 7004 | `cells_query` | returns 2 (A3) |
| 7010–7014 | `kv_get`, `kv_put`, `kv_delete`, `kv_mark_secret`, `delete_on_consume` | returns 2 until the storage phase |
| 7020 | `sign(msg32, sig_out64)` | returns 2 until the storage/sign phase |
| 7030 | `tx_skeleton_put(addr, len)` | stored (used in A3) |
| 7031 | `result_put(addr, len)` | implemented; more than 65,536 bytes stops the run with `result_too_large` |
| 7050, 7051 | `random_bytes`, `prekey_decrypt` | reserved, return 2 |

Unassigned numbers inside 7001–7099 return 2. Numbers outside CKB's set and 7001–7099 stop the run with `unknown_syscall`.

## Result

`result_put` bytes; if the service never calls it, the last debug line of the form `CSR-RESULT:0x<hex>`; else empty.
The result is only reported for statuses `ok` and `script_error`.

## Limits

Cycles (`limits.max_cycles_per_job`, `--max-cycles`), memory (CKB-VM's fixed 4 MiB; any other value is refused),
wall time (`limits.max_job_wall_seconds`, `--max-wall-seconds`; a watchdog thread sets ckb-vm's pause flag, which also stops loops without syscalls).
