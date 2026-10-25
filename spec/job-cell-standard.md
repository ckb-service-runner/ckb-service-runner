# CSR job cell standard (`csr-job-type`, `csr-job-lock`)

Status: contracts v3 (registry v3 dep, attestation check, refund at the job input's index; decisions D21 A,
D24 A, findings F-CON-003, F-CON-007). Earlier: A3a with the security fixes (audit F2, F5–F7). Reference:
CSR-SPEC-v0.2 §9/§10, deployment plan §3.3–3.5.

## 1. Job cell

* Type script `csr-job-type`, args = `service_code_hash (32)`.
* Lock script `csr-job-lock`, args = `registry_type_hash (32)` – the hash of the `csr-registry` type script
  (identical for all registry cells of one deployment). The registry cell dep holds **`RegistryV3`**
  (`registry.md` §2).
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

**One job input or output per service group (F-CON-007, by design).** A script group is all cells with the
same script; for `csr-job-type` that is one service (`service_code_hash`, the type args). A transaction may
therefore carry **at most one job input and at most one job output of the same service**: creating two jobs of
one service, or settling two jobs of one service, in one transaction is rejected with error 9
(`job_type_two_new_jobs_one_group_rejected`). Batching jobs of one service in one transaction is **not
supported**; use one transaction per job (the job lock also allows only one job input per transaction, error 7).

**Job lock** (one job input per transaction):

* *One job input*: no other input may be locked by the same job-lock code hash (any args, any data hash type),
  so two jobs can never share one tip (error 7). A deployment that references the lock both by data hash and
  by type ID is not covered; CSR references it by data hash only.
* *Refund path*: the job input's `since` is an absolute block number ≥ `expiry_block`, and an input with lock
  hash `refund_lock_hash` is present → anything allowed.
* *Run path* (taken when the refund conditions do not hold **and** a registry cell dep is present):
  1. The trigger holds (table above).
  2. Exactly one cell dep with type hash `registry_type_hash` exists; its data must be a valid **`RegistryV3`**
     (10 fields, `version = 3`; a v1/v2 table is error 10); its `payout_lock_hash` = F, and
     Σ capacity(outputs with lock hash F) − Σ capacity(inputs with lock hash F) ≥ `tip_shannons`.
  3. *Registered runner* (F2): the registry cell dep's **lock** (a `csr-dual-lock`) must parse as a script with
     **64- or 32-byte** args (else 10); its runner lock hash R = the last 32 bytes (`args[32..64]`, or the
     whole 32-byte args of a runner-owned cell). An input with lock hash R must exist – the node's runner key
     cell, so the transaction is signed with the registered service key (else 15).
  4. *Valid attestation* (F5, D21 A, D24 A): `config_id` = the registry cell dep's **type script args** (32
     bytes, else 10). Exactly one cell dep has the type hash of `csr-config-type` (hash type `data2`, code hash
     embedded at build time) with `args = config_id` – the monitor config/clock cell (else 16; its data must be
     at least `monitor_pubkey (33) ‖ clock_ms (u64 LE)`, shorter data is also 16). The registry's
     `attestation_sig` must recover to `monitor_pubkey` over `CSR-ATTEST-v2` of the registry's fields with
     `registry_type_hash` = this lock's args (else 10); `expiry_ms > clock_ms` (else 17) and
     `expiry_ms − clock_ms ≤ 8 days` (else **18**, "attestation lifetime over the cap"). The registry type
     script verified the same when the cell was written; the job lock checks it again so that it does not
     depend on how the dep came to exist.
  5. No siphoning, **at the same index** (F-CON-003): let *i* be the index of the job input among the
     transaction's inputs.
     * If `remaining_runs > 1` there is exactly one output with the same job lock and job type (byte-identical
       scripts) (else 12); it must be **output *i*** (else **19**); its data equals the input data except
       `remaining_runs − 1`, and its capacity ≥ input capacity − `tip_shannons` − `max_fee_shannons` (else 12).
     * If `remaining_runs ≤ 1` the job cell is consumed: Σ capacity(outputs with `refund_lock_hash`) −
       Σ capacity(inputs with `refund_lock_hash`) ≥ input capacity − `tip_shannons` − `max_fee_shannons` (else
       13), and **output *i*** has lock hash `refund_lock_hash` and on its own at least that capacity (else
       **19**).

     Why (F-CON-003): the "one job input" rule only counts inputs of the **same** job-lock code. Jobs of two
     job-lock versions (two deployments, or the old and the new code during a migration) in one transaction
     each saw one job input and could both be satisfied by one shared refund or one shared recreated output.
     With the output at the job input's own index, two jobs need two outputs
     (`job_two_lock_versions_share_refund_rejected`).
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
trigger (8/9), tip (11), registered runner (10/15), attestation (10/16/17/18), recreation/refund
(12/19 for a recreation, 13/19 for a refund).

### Limits (by design)

* A run is bound to a registered node (rule 3), but not to a particular one: any node with a live registry
  cell and a fresh attestation may run any job and collects the tip in its own payout cell. Results are off-chain
  (part c).
* Attestation freshness is only as good as the monitor's `clock_ms` updates (rule 4): the clock cell is the
  upper bound for "now"; a stale clock keeps an attestation valid longer, but never more than 8 days beyond
  the clock (D24 A), and `csr-config-type` lets the clock only grow.
* The stake is not checked on a run (D22 C, `registry.md` §4.1): a registration whose deposit was withdrawn
  keeps earning until its attestation expires (at most 7 days as issued, 8 days by the cap).
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
| 10 | more than one registry cell dep, registry data not valid `RegistryV3`, or its attestation signature not by the monitor over `CSR-ATTEST-v2` with this lock's `registry_type_hash` |
| 11 | tip not paid to the registered payout cell |
| 12 | recreated job cell missing / wrong (count, data, capacity) |
| 13 | refund too small |
| 14 | neither path: no registry cell dep and the refund conditions (`since` absolute block ≥ `expiry_block`, `refund_lock_hash` input) do not hold |
| 15 | `ERR_NOT_REGISTERED_RUNNER`: no input with the registry dep's runner lock hash (last 32 bytes of its args) |
| 16 | `ERR_CONFIG_DEP`: not exactly one config cell dep (`csr-config-type`, `data2`, args = the registry's `config_id`), or its data shorter than 41 bytes |
| 17 | `ERR_ATTESTATION_EXPIRED`: registry `expiry_ms ≤ clock_ms` of the config cell |
| 18 | `ERR_LIFETIME_CAP`: attestation lifetime over the cap, `expiry_ms − clock_ms` > 8 days |
| 19 | `ERR_OUTPUT_INDEX`: the refund (last run) or the recreated job cell (`Every`) is not at the job input's index, or the refund output at that index is short |

Error 10 additionally covers a registry dep whose lock args are neither 32 nor 64 bytes or whose type args are
not 32 bytes.

On a run with a forbidden recreation both scripts would fail; CKB runs lock groups before type groups, so the
job lock's code (12) is reported.

## 5. Worked examples

The refund or the recreated job cell sits at the **same index** as the job input; CSR puts the job input at
index 0 and its refund / recreated job at output 0.

*OnDemand run, single.* Inputs `[job cell (since 0), payout cell, runner key cell]`, outputs `[refund cell
(capacity − tip − fee), payout cell (+tip), runner key cell]`, cell deps `[registry cell, monitor config cell,
code deps]`. The runner key cell (lock hash = the registry's runner lock hash) is signed with the service key.
Since A8 the payout cell is the runner wallet: inputs `[job cell, runner wallet cell]`, outputs `[refund cell,
runner wallet cell (+tip)]`.

*Every(10) run.* Inputs `[job cell (since = relative 10 blocks), payout cell, runner key cell]`, outputs
`[job cell with `remaining_runs − 1` and capacity ≥ in − tip − max_fee, payout cell (+tip), runner key cell]`.

*Last Every run.* As above with `remaining_runs == 1`: no recreated job cell, a refund cell at output 0 instead.

*Rejected:* the refund at another output than the job input's index (19, `job_lock_refund_at_other_index_rejected`);
one job of each of two job-lock builds with one shared refund (the second job's lock: 19,
`job_two_lock_versions_share_refund_rejected`); a registry dep whose attestation runs more than 8 days beyond the
clock (18, `job_lock_attestation_lifetime_over_cap_rejected`).

*Refund after expiry.* Inputs `[job cell (since = absolute block ≥ expiry_block), refund-lock cell]`, outputs
anything; no registry or config cell dep needed.

Every run transaction carries both the registry cell and the monitor config cell as cell deps.

## 6. Cycles

Measured with ckb-testtool (release build, `opt-level = "s"`; job-type binary 9,320 bytes, job-lock 93,680 bytes
since v3 includes `k256`), contracts v3:

| transaction | cycles |
|-------------|-------:|
| OnDemand run, `csr-job-lock` alone (no job type; runner key cell and wallet are an always-success cell) | 7,935,959 |
| OnDemand run, whole tx (job lock + job type + always-success payout + system lock) | 9,629,047 |
| Every run with recreation, whole tx | 9,685,387 |
| settlement into the runner wallet, whole tx | 9,622,456 |

Before v3 (no signature check in the job lock): 51,406 / 1,779,894 / 1,780,935. The attestation signature
recovery (D21 A) costs ~7.9M cycles per run, well below the 70M limit.
