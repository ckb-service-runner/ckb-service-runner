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

The trigger is proven by the **`since` of the job input** (the lock reads it with `load_input_since`):

| trigger | required `since` of the job input |
|---------|------------------------------------|
| `OnDemand` | none |
| `AtBlock(n)` | absolute block number ≥ n |
| `AtEpoch(e)` | absolute epoch (packed `number‖index‖length`) ≥ e |
| `AtTimestamp(t)` | absolute median-time timestamp ≥ t (`t` in ms; since uses seconds, so `since_secs*1000 ≥ t`) |
| `Every(k)` | **relative** block number ≥ k |

`OnCellConsumed` from the spec **cannot be proven on-chain** (a script cannot prove that a cell no longer
exists), so it is an off-chain trigger only: the runner watches it and submits an `OnDemand` job.

## 3. Rules

**Creation** (`csr-job-type`, group output without input): `remaining_runs ≥ 1`; `Every` allows any
`remaining_runs`, every other trigger requires `remaining_runs == 1`; capacity ≥ occupied capacity +
`remaining_runs × (tip_shannons + max_fee_shannons)`.

**Update** (group input and output; only reachable for `Every`, since the lock forces it): every field
identical except `remaining_runs` = input − 1.

**Consume** (group input without output): allowed by the type script; the lock decides.

**Job lock** (one job input per transaction):

* *Refund path*: the job input's `since` is an absolute block number ≥ `expiry_block`, and an input with lock
  hash `refund_lock_hash` is present → anything allowed.
* *Run path*:
  1. The trigger holds (table above).
  2. A cell dep with type hash `registry_type_hash` exists; its data's `float_lock_hash` = F, and
     Σ capacity(outputs with lock hash F) − Σ capacity(inputs with lock hash F) ≥ `tip_shannons`.
  3. No siphoning: if `remaining_runs > 1` there is exactly one output with the same job lock and job type,
     its data equals the input data except `remaining_runs − 1`, and its capacity ≥ input capacity −
     `tip_shannons` − `max_fee_shannons`. If `remaining_runs == 1` the job cell is consumed and
     Σ capacity(outputs with `refund_lock_hash`) ≥ input capacity − `tip_shannons` − `max_fee_shannons`.

The **fee is paid from the job's capacity** (capped by `max_fee_shannons`); the float receives at least the
tip. (This replaces "float pays the fee, job pays it back" from spec §3.3.)

## 4. Error codes

`csr-job-type`:

| code | meaning |
|-----:|---------|
| 5 | args are not 32 bytes |
| 6 | job data not valid molecule |
| 7 | `remaining_runs` invalid for the trigger |
| 8 | capacity below occupied + runs × (tip + max_fee) |
| 9 | update of a non-`Every` job, or more than one input/output in the group |
| 10 | update changed a field other than `remaining_runs` / wrong decrement |

`csr-job-lock`:

| code | meaning |
|-----:|---------|
| 5 | args are not 32 bytes |
| 6 | job data not valid molecule |
| 7 | not exactly one job input |
| 8 | `since` has the wrong form (flags) for the trigger |
| 9 | trigger not reached |
| 10 | registry cell dep missing or not unique |
| 11 | tip not paid to the registered float |
| 12 | recreated job cell missing / wrong |
| 13 | refund too small |
| 14 | refund path without `refund_lock_hash` input, or `since` below `expiry_block` |

## 5. Worked examples

*OnDemand run, single.* Inputs `[job cell (since 0), runner key cell]`, outputs `[float cell (+tip), refund
cell (capacity − tip − fee), runner key cell]`, cell deps `[registry cell, code deps]`.

*Every(10) run.* Inputs `[job cell (since = relative 10 blocks), runner key cell]`, outputs `[job cell with
`remaining_runs − 1` and capacity ≥ in − tip − max_fee, float (+tip), runner key cell]`.

*Refund after expiry.* Inputs `[job cell (since = absolute block ≥ expiry_block), owner cell]`, outputs
anything.
