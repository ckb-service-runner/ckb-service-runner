# CSR job cell standard (`csr-job-type`, `csr-job-lock`)

Status: A3a, with the A3a security fixes (audit F2, F5–F7). Reference: CSR-SPEC-v0.2 §9/§10, deployment plan §3.3–3.5.

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
exists). It is an `OnDemand` job with a watch convention (decision D2 A): the payload starts with the 41-byte
header `"CSRW"` ‖ version `01` ‖ the watched out point (tx hash ‖ index as u32 LE). The runner leaves such a job
alone until it has seen the watched cell consumed, then runs it like any other job. On chain it is plain
`OnDemand`, so the chain cannot check that the event happened: a runner that does not follow the convention
could run the job earlier. The service receives the whole payload, including the header. Watch headers on
other triggers are ignored.

Example header for cell `0xaa…aa:1` (tx hash of 32 × `aa`, index 1):

```
43535257 01 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 01000000
```

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
  2. Exactly one cell dep with type hash `registry_type_hash` exists; its data's `payout_lock_hash` = F, and
     Σ capacity(outputs with lock hash F) − Σ capacity(inputs with lock hash F) ≥ `tip_shannons`.
  3. *Registered runner* (F2): the registry cell dep's **lock** (a `csr-dual-lock`) must parse as a script with
     64-byte args (else 10); its runner lock hash R = `args[32..64]`. An input with lock hash R must exist – the
     node's runner key cell, so the transaction is signed with the registered service key (else 15).
  4. *Live attestation* (F5): `config_type_hash` = the registry cell dep's **type script args** (32 bytes, else
     10). Exactly one cell dep with type hash `config_type_hash` exists – the monitor config/clock cell (else
     16; its data must be at least `monitor_pubkey (33) ‖ clock_ms (u64 LE)`, shorter data is also 16). The
     registry data's `attestation.expiry_ms > clock_ms` (else 17). The attestation signature is **not**
     verified again: the registry type script verified it when the cell was written.
  5. No siphoning: if `remaining_runs > 1` there is exactly one output with the same job lock and job type
     (byte-identical scripts), its data equals the input data except `remaining_runs − 1`, and its capacity ≥
     input capacity − `tip_shannons` − `max_fee_shannons`. If `remaining_runs ≤ 1` the job cell is consumed and
     Σ capacity(outputs with `refund_lock_hash`) − Σ capacity(inputs with `refund_lock_hash`) ≥ input capacity
     − `tip_shannons` − `max_fee_shannons`.
* *Neither*: no registry cell dep and the refund conditions do not hold → error 14. The set of accepted
  transactions is exactly "refund path or run path"; the registry cell dep only selects which error is
  reported (a refund transaction never carries a registry dep, a run always does).

Both capacity sums are **net** (outputs − inputs with that lock hash), so a cell of F or of the refund lock
that is spent and recreated in the same transaction (e.g. the payout cell, or an anyone-can-pay refund cell)
counts only with the amount actually added.

The **fee is paid from the job's capacity** (capped by `max_fee_shannons`); the payout cell receives at least the
tip. The payout cell is a standard anyone-can-pay cell (lock `args` = 20 bytes, no minimum, no type, empty
data), so a run transaction either adds the tip to the existing cell (inputs `[job, payout cell, runner key
cell]`; the ACP lock needs no signature when the output has at least the input's capacity and the witness is
empty) or creates a new cell with the same lock. CSR defines no slashing and no job collateral; applications
that need a bond define it in their own contracts.

Rule order (determines the reported code): job inputs (7), data (6), refund path, registry dep (14/10),
trigger (8/9), tip (11), registered runner (10/15), live attestation (10/16/17), recreation/refund (12/13).

### Limits (by design)

* A run is bound to a registered node (rule 3), but not to a particular one: any node with a live registry
  cell and a fresh attestation may run any job and collects the tip in its own payout cell. Results are off-chain
  (part c).
* Attestation freshness is only as good as the monitor's `clock_ms` updates (rule 4): the clock cell is the
  upper bound for "now"; a stale clock keeps an attestation valid longer.
* **F6 – fee may go to any output (documented, no code change).** On the last run (and on every run, for the
  recreated job's capacity) up to `max_fee_shannons` of the job capacity is not bound to the miner fee: the
  transaction builder (the runner) may send it to any output, e.g. its own cell. This is bounded by what the
  consumer chose in `max_fee_shannons` (and prepaid on creation, `runs × (tip + max_fee)`); consumers should
  set `max_fee_shannons` to a realistic fee, not a large allowance.
* **F7 – job lock without job type (documented, no code change).** The job lock does not require the job input
  to carry `csr-job-type`. A cell that has only the job lock skips the creation checks (funding, run counts)
  and the update rules of the type script; that is the creator's own risk (e.g. `remaining_runs = 0`, an
  underfunded cell, or several runs of a non-`Every` trigger). CSR discovers jobs by job type **and** job lock
  and ignores cells with only one of them.
* **F8 – several registry cells per key (see `registry.md`).** One key holder may create several registry
  cells; the job lock accepts any one of them as the dep (each must pass rules 2–4). Off-chain consumers
  (CSR, monitor, CRT) pick the one with the latest attestation.
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
| 11 | tip not paid to the registered payout cell |
| 12 | recreated job cell missing / wrong (count, data, capacity) |
| 13 | refund too small |
| 14 | neither path: no registry cell dep and the refund conditions (`since` absolute block ≥ `expiry_block`, `refund_lock_hash` input) do not hold |
| 15 | `ERR_NOT_REGISTERED_RUNNER`: no input with the registry dep's runner lock hash (`args[32..64]`) |
| 16 | `ERR_CONFIG_DEP`: not exactly one cell dep with the registry's `config_type_hash`, or its data shorter than 41 bytes |
| 17 | `ERR_ATTESTATION_EXPIRED`: registry `attestation.expiry_ms ≤ clock_ms` of the config cell |

Error 10 additionally covers a registry dep whose lock args are not 64 bytes or whose type args are not 32
bytes.

On a run with a forbidden recreation both scripts would fail; CKB runs lock groups before type groups, so the
job lock's code (12) is reported.

## 5. Worked examples

*OnDemand run, single.* Inputs `[job cell (since 0), payout cell, runner key cell]`, outputs `[float cell
(+tip), runner key cell, refund cell (capacity − tip − fee)]`, cell deps `[registry cell, monitor config cell,
code deps]`. The runner key cell (lock hash = the registry's runner lock hash) is signed with the service key.

*Every(10) run.* Inputs `[job cell (since = relative 10 blocks), payout cell, runner key cell]`, outputs
`[payout cell (+tip), runner key cell, job cell with `remaining_runs − 1` and capacity ≥ in − tip − max_fee]`.

*Last Every run.* As above with `remaining_runs == 1`: no recreated job cell, a refund cell instead.

*Refund after expiry.* Inputs `[job cell (since = absolute block ≥ expiry_block), refund-lock cell]`, outputs
anything; no registry or config cell dep needed.

Every run transaction carries both the registry cell and the monitor config cell as cell deps.

## 6. Cycles

Measured with ckb-testtool (release build, `opt-level = "s"`; job-type binary ~9.3 KB, job-lock ~12.9 KB),
after the security fixes (registered-runner and attestation-expiry checks):

| transaction | cycles |
|-------------|-------:|
| OnDemand run, `csr-job-lock` alone (no job type; runner key cell and wallet are an always-success cell) | 51,406 |
| OnDemand run, whole tx (job lock + job type + anyone-can-pay lock + system lock) | 1,779,894 |
| Every run with recreation, whole tx | 1,780,935 |

Before the fixes: 38,219 / 1,738,768 / 1,794,708. The extra checks cost ~13 k cycles (one more cell-dep scan,
the registry lock/type loads); the system lock's secp256k1 verification still dominates.

## Run lifecycle

This section describes what a runner (the keeper of `csr`) does with a job cell. It is not part of the
on-chain rules above.

| State | Meaning | Next |
|---|---|---|
| `Discovered` | run created for a selected job cell | `Queued` |
| `Queued` | waits for the next batch | `Scheduled`, `Paused`, `Lost`, `Expired` |
| `Scheduled` | in the batch about to start | `Running`, `Paused` |
| `Paused` | held (R2 or resource hold, or an unusable attestation); not started yet | `Queued`/`Scheduled` again, `Lost`, `Expired` |
| `Running` | the service runs in the sandbox | `Completed`, `Failed` |
| `Completed` | executed, transaction not yet sent | `Submitting`, `Failed` |
| `Submitting` | transaction sent, waits for commit | `Settled`, `Rejected`, `Lost` |
| `Settled` | transaction committed | final |
| `Lost` | the job cell was spent by someone else | final |
| `Expired` | the job reached its expiry before it ran | final |
| `Failed` | execution or transaction building failed | final for this run |
| `Rejected` | the node refused or dropped the transaction while the job cell is live | final for this run |

**Attempts.** A job cell gets at most three runs. `Failed` and `Rejected` runs count; after the third the
job cell is not tried again.

**Give-up timers.** A submitted transaction that the node no longer knows is given up after 30 blocks; a
transaction still pending is given up after 500 blocks. A runner wallet sweep that is still pending is
given up after 500 blocks as well. A given-up run becomes `Rejected` while the job cell is live and `Lost`
otherwise.

**Restart.** When the runner starts, runs in `Running`, `Scheduled`, `Paused` or `Completed` go back to
`Queued`; `Submitting` runs are followed on.

**Batch abort.** If executing or submitting a batch fails as a whole, the runs of that batch still in
`Running`, `Completed` or `Scheduled` become `Failed` (one attempt each), so the next batch can start.

**Attestation.** No run starts while the runner's registry attestation is expired (its `expiry_ms` is not
after the chain clock of the config cell). The runs stay waiting and the jobs page shows the reason.

**Pause rule.** The R2 and resource holds pause only runs that have not started yet. A run that has
started finishes, within `max_job_wall_seconds`.

A service must tolerate being run more than once for one job cell.
