# Milestone A3, part a – On-chain scripts

## Built
- Separate cargo workspace `contracts/` with five `ckb-std` contracts (no_std, `riscv64imac-unknown-none-elf`):
  - `csr-float-lock` – owner path / runner path (one identical float output, fee ≤ `max_fee_shannons`, merges allowed).
  - `csr-dual-lock` – unlocks with an owner or runner key cell; only used under the registry type script.
  - `csr-registry` – type script; monitor attestation verified in-script (secp256k1 recovery with `k256`) against a
    Type ID config cell dep (`monitor_pubkey ‖ clock_ms`); owner/runner update rules, owner-only destroy.
  - `csr-job-type` – job creation funding (`occupied + runs × (tip + max_fee)`), run counts per trigger, Every update.
  - `csr-job-lock` – triggers proven by `since`, tip to the float named by a registry cell dep, anti-siphon
    (recreation or refund), refund path after `expiry_block`.
- Molecule schema `contracts/schemas/csr.mol` (generated Rust in `csr-schemas`, used by the tests; the contracts
  parse molecule by hand to avoid the `bytes` crate).
- `scripts/build-contracts.sh` → `contracts/build/release/<name>`.
- `csr-contract-tests`: ckb-testtool harness with the real system secp256k1 lock, a Type ID config cell, real occupied
  capacities and signed transactions; every rule has an `_ok` and a `_rejected` test that asserts the failing script
  and its error code (83 tests: harness 1, float 10, registry/dual lock 30, job 42).
- Standards: `spec/float-lock.md`, `spec/registry.md` (attestation byte layout `CSR-ATTEST-v1`), `spec/job-cell-standard.md`.

## Demo
```
bash scripts/build-contracts.sh
cargo test --manifest-path contracts/Cargo.toml -p csr-contract-tests -- --test-threads=1 --nocapture
node acceptance/a3a/named-tests.mjs harness float registry job
```

## Cycles (ckb-testtool, release build)
| what | cycles |
|------|-------:|
| registry creation, type script alone | ~7.9 M |
| registry creation / runner refresh, whole tx | ~9.6 M |
| job run, job lock alone | ~38 k |
| OnDemand job run, whole tx (job lock + job type + float lock + system lock) | ~1.7 M |

## Limits found
- On-chain time is only lower-bounded (`since`, header deps); the live monitor config cell is the upper bound for
  "now", so attestation freshness is only as good as the monitor's `clock_ms` updates.
- `OnCellConsumed` cannot be proven on-chain; it is an off-chain trigger submitted as `OnDemand`.
- (Fixed by the security fixes below: the job lock now binds a run to the registered runner key and checks
  attestation expiry.) Runs after `expiry_block` stay possible until refunded.
- ckb-testtool does not verify `since` against a chain; the tests cover the scripts' own `since` checks.
- `csr-registry` embeds the code hash of the built `csr-dual-lock`; a changed dual lock needs a new registry binary.
- `ckb-types`/molecule's `bytes` crate emits atomics that ckb-vm rejects, so contracts use only `ckb_std::syscalls`.
- No CSR binary changes yet: part b wires the scripts to the chain, part c runs jobs.
- See `docs/decisions.md` (A3a phases 1–4) for details.

## Security fixes
An audit found that the scripts implement the A3a spec faithfully, but the spec left holes. Fixed (with exploit
tests that assert the failing script and the exact error code):

| finding | severity | fix | error code |
|---------|----------|-----|-----------:|
| F1 registry entries without the service key | critical | `csr-registry`: creation, and owner-path change of `service_pubkey`/runner lock, require an input with the output's runner lock hash (the runner key cell) | registry 17 `ERR_NO_RUNNER_KEY` |
| F2 anyone can execute jobs | critical | `csr-job-lock` run path: input with the registry dep's runner lock hash (dual-lock `args[32..64]`) required | job lock 15 `ERR_NOT_REGISTERED_RUNNER` |
| F3 stolen key freezes the float | high | `csr-float-lock` runner path: float output has no type script and empty data | float 10 `ERR_OUTPUT_SHAPE` |
| F4 stolen key drains the float | high | `csr-float-lock` runner path: no capacity loss at all; `max_fee_shannons` reserved | float 8 |
| F5 expired attestations keep earning | medium | `csr-job-lock` run path: exactly one config cell dep (`config_type_hash` from the registry type args), `expiry_ms > clock_ms` | job lock 16 `ERR_CONFIG_DEP`, 17 `ERR_ATTESTATION_EXPIRED` |
| F6 fee allowance may go to any output | low | documented (`spec/job-cell-standard.md` §3), bounded by the consumer's `max_fee_shannons` | – |
| F7 job lock without job type | low | documented: creator's own risk; CSR discovers jobs by type and lock | – |
| F8 several registry cells per key | low | documented (`spec/registry.md`): consumers pick the latest attestation | – |

Tests after the fixes: 98 (harness 1, float 13, registry/dual lock 35, job 49).

Cycles after the fixes: job lock alone 51,406 (was 38,219); OnDemand run whole tx 1,779,894; Every run with
recreation 1,780,935 (see `spec/job-cell-standard.md` §6).

Downstream impact (see `docs/decisions.md`): `csr register prepare` (A3b) adds and signs the runner key cell; job
transactions (A3c) add the monitor config cell as a cell dep and the runner key cell as input; the float never
pays fees.
