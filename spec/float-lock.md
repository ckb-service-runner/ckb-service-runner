# CSR float lock (`csr-float-lock`) and dual lock (`csr-dual-lock`)

Status: A3a, audit fixes F3/F4 applied (A3a-fix Phase 1). Reference: CSR-SPEC-v0.2 §9/§10, deployment plan §3.3–3.5.

## 1. Authorisation by key cell

No CSR lock verifies an ECDSA signature itself. A path is **authorised when the transaction has an input whose
lock hash equals the authorising lock hash** (`load_cell_by_field(.., Source::Input, CellField::LockHash)`).
That input's own lock (normally `secp256k1_blake160_sighash_all`, but any lock: Neuron, JoyID, multisig) does
the signature check.

* **Owner lock hash** – lock hash of the operator's wallet (any lock).
* **Runner lock hash** – lock hash of `secp256k1_blake160_sighash_all` with `args = blake160(service pubkey)`.
  CSR keeps a small **runner key cell** under this lock and spends/recreates it in each transaction that uses
  the runner path.

Consequence: the lock only proves "someone who can unlock lock hash X agreed to this transaction"; it
therefore must be combined with rules on the outputs (below) wherever the runner path exists.

## 2. `csr-float-lock`

Script args (72 bytes, no molecule):

| offset | size | field |
|-------:|-----:|-------|
| 0 | 32 | `owner_lock_hash` |
| 32 | 32 | `runner_lock_hash` |
| 64 | 8 | `max_fee_shannons` (u64 LE) – **reserved**, ignored by the script (see below) |

Any other length → error `5`.

Rules (evaluated once per script group, over all float inputs of the group):

1. **Owner path**: an input with lock hash `owner_lock_hash` exists → accepted, anything allowed.
2. **Runner path** (otherwise): an input with lock hash `runner_lock_hash` must exist, and
   * the transaction has **exactly one** output whose lock is *identical* to this script (same code hash, hash
     type, args);
   * that output has **no type script** and **empty data** (error `10`; float cells never carry either);
   * that output's capacity ≥ Σ(capacity of the float-lock inputs in this script group) – **no loss at all**
     (error `8`). Several float inputs may be merged into that one output.
3. Neither key cell present → rejected.

Why (audit findings, `docs/decisions.md`):

* **F3** – without the shape rule a stolen service key could put an always-failing (or attacker-controlled)
  type script, or data that such a type would need, on the float output; the cell could then never be spent
  again, not even by the owner (freeze/ransom).
* **F4** – the former bound "output ≥ inputs − `max_fee_shannons`" let a stolen service key move up to
  `max_fee_shannons` per transaction to any output, with no job involved; repeated, this drains the float.
  Job fees are paid from the job cell, so the float never has to pay a fee and the runner path now allows no
  loss. `max_fee_shannons` stays in the args (72 bytes, layout unchanged so existing float lock hashes,
  CSR and tests keep their format) but is **reserved/unused**; write `0` or any value, the script ignores it.

A runner transaction that spends the float must therefore pay its miner fee from another input (the runner key
cell or the job cell).

### Error codes

| code | meaning |
|-----:|---------|
| 1–4 | ckb-std syscall errors (IndexOutOfBound, ItemMissing, LengthNotEnough, Encoding) |
| 5 | args length is not 72 |
| 6 | neither owner nor runner key cell present |
| 7 | runner path: number of outputs with the float lock is not exactly 1 |
| 8 | runner path: output capacity below Σ float inputs (any loss) |
| 9 | capacity arithmetic overflow |
| 10 | runner path: float output has a type script or non-empty data |

### Worked examples

*Runner pays a tip into the float (OK).* Inputs: `[runner key cell (10 CKB), job cell…]`; outputs:
`[float cell: 1000 CKB + tip, runner key cell (10 CKB − fee)]`. The float cell is **not** an input here: a
deposit does not unlock the float, the float lock only runs when a float cell is spent.

*Runner spends float (OK).* Inputs: `[float (1000 CKB), runner key cell (10 CKB)]`; outputs: `[float, same
lock, no type, empty data, ≥ 1000 CKB; runner key cell (10 CKB − fee)]`. The fee comes from the key cell.

*Merge (OK).* Inputs `[float A, float B, runner key]`, outputs `[one float with identical lock ≥ A+B, runner
key cell − fee]`. Both inputs are in the same group and the script runs once.

*Owner withdraw (OK).* Inputs `[float, owner wallet cell]`, outputs anything.

*Rejected:* float output 1 shannon below the input (8, `float_runner_capacity_loss_rejected`); runner key +
float only, float shrinks by `max_fee_shannons` into the key cell (8, `float_runner_no_job_drain_rejected`);
type script on the float output (10, `float_runner_adds_type_script_rejected`); data on the float output (10,
`float_runner_sets_data_rejected`); output to another lock (7); two float outputs (7); changed args in the
output (7, because the identical lock is not found); no key cell (6).

## 3. `csr-dual-lock`

Script args (64 bytes): `owner_lock_hash (32) ‖ runner_lock_hash (32)`.

Unlocks if an input with lock hash `owner_lock_hash` **or** `runner_lock_hash` is present. It is used only
together with a type script that decides what each path may do (the registry, see `registry.md`); on its own
it is not safe.

| code | meaning |
|-----:|---------|
| 5 | args length is not 64 |
| 6 | neither key cell present |

Worked example: a registry cell (`lock = dual-lock`) is spent with inputs `[registry cell, runner key cell]` →
the lock passes, the registry type script enforces runner-path rules.

## 4. Use in job runs

A job run (see `job-cell-standard.md`) pays the tip by **merging it into an existing float cell**: inputs
`[job, float, runner key cell]`, outputs `[float + tip, runner key cell, …]`. The float lock takes its runner
path (exactly one identical float output without type/data, capacity ≥ input, here input + tip), and the job
lock counts the **net** inflow to the float lock hash (outputs − inputs), so spending and recreating the float
in the same transaction is fine. The transaction fee is paid from the job cell (≤ the job's
`max_fee_shannons`), never from the float (the float lock forbids any loss on the runner path). A first tip can also be a plain deposit (a new float cell), which does
not run the float lock at all.

Implementation note: the float lock uses only `ckb_std::syscalls` (no `high_level`), like all CSR contracts,
because the `ckb-types` feature is not enabled in the contracts' build (see `docs/decisions.md`).
