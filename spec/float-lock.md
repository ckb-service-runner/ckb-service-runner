# CSR float lock (`csr-float-lock`) and dual lock (`csr-dual-lock`)

Status: A3a. Reference: CSR-SPEC-v0.2 §9/§10, deployment plan §3.3–3.5.

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
| 64 | 8 | `max_fee_shannons` (u64 LE) |

Any other length → error `5`.

Rules (evaluated once per script group, over all float inputs of the group):

1. **Owner path**: an input with lock hash `owner_lock_hash` exists → accepted, anything allowed.
2. **Runner path** (otherwise): an input with lock hash `runner_lock_hash` must exist, and
   * the transaction has **exactly one** output whose lock is *identical* to this script (same code hash, hash
     type, args);
   * that output's capacity ≥ Σ(capacity of the float-lock inputs in this script group) − `max_fee_shannons`.
     Several float inputs may be merged into that one output. The output's type script and data are not
     restricted (a float cell is plain capacity).
3. Neither key cell present → rejected.

The `max_fee_shannons` bound is what stops a compromised runner key from draining the float: per transaction
at most `max_fee_shannons` leaves the float cell(s) (it may only go to fees/other outputs), the rest must
return to the same float lock.

### Error codes

| code | meaning |
|-----:|---------|
| 1–4 | ckb-std syscall errors (IndexOutOfBound, ItemMissing, LengthNotEnough, Encoding) |
| 5 | args length is not 72 |
| 6 | neither owner nor runner key cell present |
| 7 | runner path: number of outputs with the float lock is not exactly 1 |
| 8 | runner path: output capacity below Σ inputs − `max_fee_shannons` |
| 9 | capacity arithmetic overflow |

### Worked examples

*Runner pays a tip into the float (OK).* Inputs: `[runner key cell (10 CKB), job cell…]`; outputs:
`[float cell: 1000 CKB + tip, runner key cell (10 CKB − fee)]`. The float cell is **not** an input here: a
deposit does not unlock the float, the float lock only runs when a float cell is spent.

*Runner spends float (OK).* Inputs: `[float (1000 CKB), runner key cell]`; outputs: `[float, same lock,
≥ 1000 CKB − max_fee; runner key cell]`.

*Merge (OK).* Inputs `[float A, float B, runner key]`, outputs `[one float with identical lock ≥ A+B −
max_fee]`. Both inputs are in the same group and the script runs once.

*Owner withdraw (OK).* Inputs `[float, owner wallet cell]`, outputs anything.

*Rejected:* runner path with fee above `max_fee` (8); output to another lock (7); two float outputs (7);
changed args in the output (7, because the identical lock is not found); no key cell (6).

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
