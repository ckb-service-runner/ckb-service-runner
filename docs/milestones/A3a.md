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
- The job lock does not re-check registry attestation expiry and does not bind a run to a runner key; anyone can
  submit a run, but the tip only goes to a registered float. Runs after `expiry_block` stay possible until refunded.
- ckb-testtool does not verify `since` against a chain; the tests cover the scripts' own `since` checks.
- `csr-registry` embeds the code hash of the built `csr-dual-lock`; a changed dual lock needs a new registry binary.
- `ckb-types`/molecule's `bytes` crate emits atomics that ckb-vm rejects, so contracts use only `ckb_std::syscalls`.
- No CSR binary changes yet: part b wires the scripts to the chain, part c runs jobs.
- See `docs/decisions.md` (A3a phases 1–4) for details.
