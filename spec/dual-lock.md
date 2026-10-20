# CSR dual lock (`csr-dual-lock`)

Status: A7 (the float lock was removed, decision D-A7-1). Reference: CSR-SPEC-v0.2 §9/§10.

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

## 2. `csr-dual-lock`

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

Implementation note: the dual lock uses only `ckb_std::syscalls` (no `high_level`), like all CSR contracts, because
the `ckb-types` feature is not enabled in the contracts' build (see `docs/decisions.md`).

## 3. Tips and the payout cell

Tips are not held by a CSR lock. They go to the operator's **payout cell**, a standard anyone-can-pay (ACP)
cell; anyone, including the runner, can add CKB to it without a signature, and only the owner's secp256k1
signature withdraws. See `job-cell-standard.md` and `registry.md`.
