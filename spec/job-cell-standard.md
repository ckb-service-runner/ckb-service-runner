# CSR job cell standard (`csr-job-type`, `csr-job-lock`)

Status: A3a. Reference: CSR-SPEC-v0.2 §9/§10, deployment plan §3.3–3.5.

## 1. Job cell

* Type script `csr-job-type`, args = `service_code_hash (32)`.
* Lock script `csr-job-lock`, args = `registry_type_hash (32)` – the hash of the `csr-registry` type script
  (identical for all registry cells of one deployment).
* Data: molecule `Job` (`contracts/schemas/csr.mol`):

```
union Trigger { OnDemand, AtBlock(u64), AtEpoch(u64 packed), AtTimestamp(u64 ms), Every(u64 blocks) }
table Job {
    trigger: Trigger, tip_shannons: u64, max_fee_shannons: u64, max_cycles: u64, payload: Bytes,
    expiry_block: u64, refund_lock_hash: Byte32, remaining_runs: u32,
}
```

## 2. Triggers and `since`

The trigger is proven by the **`since` of the job input** (the lock reads it with
`load_input_by_field(Since)`); CKB consensus guarantees that a transaction with this `since` can only be
committed once the condition holds, the lock checks that the `since` has the right **form** (flags) and a
**value** at least the trigger.

| trigger | required `since` of the job input | flags byte |
|---------|------------------------------------|-----------:|
| `OnDemand` | none (any `since`, usually 0) | – |
| `AtBlock(n)` | absolute block number ≥ n | `0x00` |
| `AtEpoch(e)` | absolute epoch (packed `number‖index‖length`) ≥ e | `0x20` |
| `AtTimestamp(t)` | absolute median-time timestamp ≥ t (`t` in ms; since uses seconds, so `since_secs*1000 ≥ t`) | `0x40` |
| `Every(k)` | **relative** block number ≥ k (blocks since the job cell was created/recreated) | `0x80` |

Details:

* `since = 0` is "absolute block 0": it satisfies `AtBlock(0)` only.
* Epochs are compared as fractions `number + index/length` (cross-multiplied, no rounding). A packed epoch with
  `length = 0` is read as the whole epoch `number` (index 0, length 1), both in the trigger and in `since`.
* A `since` with the wrong flags (e.g. relative for `AtBlock`, block for `AtTimestamp`) is error 8, a value
  below the trigger is error 9.
* `ckb-testtool` does not verify `since` against the chain (it has no chain), so the tests exercise the lock's
  own check of the `since` value; the "the condition really holds" half is CKB consensus.

`OnCellConsumed` from the spec **cannot be proven on-chain** (a script cannot prove that a cell no longer
exists), so it is an off-chain trigger only: the runner watches it and submits an `OnDemand` job.

## 3. Rules

**Creation** (`csr-job-type`, group output without input): `remaining_runs ≥ 1`; `Every` allows any
`remaining_runs`, every other trigger requires `remaining_runs == 1`; capacity ≥ occupied capacity +
`remaining_runs × (tip_shannons + max_fee_shannons)`. Occupied capacity = 8 (capacity field) + lock
(32 + 1 + args) + type (32 + 1 + args) + data length, in CKBytes (10^8 shannons each); the sum is computed
without overflow (u128).

**Update** (group input and output; only reachable for `Every`, since the lock forces it): the input trigger
is `Every` (else 9); every field identical except `remaining_runs` = input − 1 ≥ 1 (else 10). Because
`remaining_runs` is the last, fixed-size field of the table, "identical except `remaining_runs`" is checked as
"same length and all bytes but the last 4 equal".

**Consume** (group input without output): allowed by the type script; the lock decides.

More than one job cell of the same job type in an input or output group is rejected (9). Job data must be a
strict molecule `Job` (exact field count, no trailing bytes; error 6).

**Job lock** (one job input per transaction):

* *One job input*: no other input may be locked by the same job-lock code hash (any args, any data hash type),
  so two jobs can never share one tip (error 7). A deployment that references the lock both by data hash and
  by type ID is not covered; CSR references it by data hash only.
* *Refund path*: the job input's `since` is an absolute block number ≥ `expiry_block`, and an input with lock
  hash `refund_lock_hash` is present → anything allowed.
* *Run path* (taken when the refund conditions do not hold **and** a registry cell dep is present):
  1. The trigger holds (table above).
  2. Exactly one cell dep with type hash `registry_type_hash` exists; its data's `float_lock_hash` = F, and
     Σ capacity(outputs with lock hash F) − Σ capacity(inputs with lock hash F) ≥ `tip_shannons`.
  3. No siphoning: if `remaining_runs > 1` there is exactly one output with the same job lock and job type
     (byte-identical scripts), its data equals the input data except `remaining_runs − 1`, and its capacity ≥
     input capacity − `tip_shannons` − `max_fee_shannons`. If `remaining_runs ≤ 1` the job cell is consumed and
     Σ capacity(outputs with `refund_lock_hash`) − Σ capacity(inputs with `refund_lock_hash`) ≥ input capacity
     − `tip_shannons` − `max_fee_shannons`.
* *Neither*: no registry cell dep and the refund conditions do not hold → error 14. The set of accepted
  transactions is exactly "refund path or run path"; the registry cell dep only selects which error is
  reported (a refund transaction never carries a registry dep, a run always does).

Both capacity sums are **net** (outputs − inputs with that lock hash), so a cell of F or of the refund lock
that is spent and recreated in the same transaction (e.g. the float, or an anyone-can-pay refund cell) counts
only with the amount actually added.

The **fee is paid from the job's capacity** (capped by `max_fee_shannons`); the float receives at least the
tip. (This replaces "float pays the fee, job pays it back" from spec §3.3.) The float lock's own runner path
also allows a fee of up to its `max_fee_shannons`, so a run transaction that merges the tip into the float
cell is accepted by both locks (see `float-lock.md`).

### Limits (by design)

* A run is not bound to a particular runner: anyone may submit a run transaction, but the tip can only go to
  a float that a live registry cell names. Results are off-chain (part c).
* The job lock does not re-check the registry cell's attestation expiry: the registry type script checks it
  on every creation/update, and a registry cell with an expired attestation still counts as registered until
  its runner refreshes it (see `registry.md`).
* Runs after `expiry_block` remain possible until somebody refunds (a script can only lower-bound time).
* `max_cycles` and `payload` are not interpreted on-chain; `max_cycles` is the runner's VM limit (part c).
* The job type args `service_code_hash` are not interpreted on-chain either: they identify the service
  that the runner executes.

## 4. Error codes

`csr-job-type`:

| code | meaning |
|-----:|---------|
| 1–4 | syscall error (index out of bound, item missing, length, other) |
| 5 | args are not 32 bytes |
| 6 | job data not valid molecule |
| 7 | `remaining_runs` invalid for the trigger |
| 8 | capacity below occupied + runs × (tip + max_fee) |
| 9 | update of a non-`Every` job, or more than one input/output in the group |
| 10 | update changed a field other than `remaining_runs` / wrong decrement |

`csr-job-lock`:

| code | meaning |
|-----:|---------|
| 1–4 | syscall error |
| 5 | args are not 32 bytes |
| 6 | job data not valid molecule |
| 7 | not exactly one job input |
| 8 | `since` has the wrong form (flags) for the trigger |
| 9 | trigger not reached |
| 10 | more than one registry cell dep, or registry data malformed |
| 11 | tip not paid to the registered float |
| 12 | recreated job cell missing / wrong (count, data, capacity) |
| 13 | refund too small |
| 14 | neither path: no registry cell dep and the refund conditions (`since` absolute block ≥ `expiry_block`, `refund_lock_hash` input) do not hold |

On a run with a forbidden recreation both scripts would fail; CKB runs lock groups before type groups, so the
job lock's code (12) is reported.

## 5. Worked examples

*OnDemand run, single.* Inputs `[job cell (since 0), float cell, runner key cell]`, outputs `[float cell
(+tip), runner key cell, refund cell (capacity − tip − fee)]`, cell deps `[registry cell, code deps]`.

*Every(10) run.* Inputs `[job cell (since = relative 10 blocks), float cell, runner key cell]`, outputs
`[float (+tip), runner key cell, job cell with `remaining_runs − 1` and capacity ≥ in − tip − max_fee]`.

*Last Every run.* As above with `remaining_runs == 1`: no recreated job cell, a refund cell instead.

*Refund after expiry.* Inputs `[job cell (since = absolute block ≥ expiry_block), refund-lock cell]`, outputs
anything; no registry cell dep needed.

## 6. Cycles

Measured with ckb-testtool (release build, `opt-level = "s"`; job-type binary ~9 KB, job-lock ~11.5 KB):

| transaction | cycles |
|-------------|-------:|
| OnDemand run, `csr-job-lock` alone (no job type, always-success wallet) | 38,219 |
| OnDemand run, whole tx (job lock + job type + float lock + system lock) | 1,738,768 |
| Every run with recreation, whole tx | 1,794,708 |

The system lock's secp256k1 verification dominates; the job scripts themselves are cheap.
