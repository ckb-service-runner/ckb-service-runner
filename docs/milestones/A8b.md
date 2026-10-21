# A8b – Polish after A8

## Phase 1: devnet faucet removed, setup figures corrected
- `csr-dev attest` no longer sends CKB to the runner wallet: no `--faucet-ckb`, no `faucet` key in the JSON, no stderr line. The attestation file is unchanged.
- Devnet flow: `csr-dev fund --lock-args <runner_lock_args> --ckb 700` (or more) before `csr register create`.
- Setup figures: send about **700 CKB**; the registry cell uses about **540 CKB**.
- `acceptance/a8b/p3`–`p5` replace the obsolete `acceptance/a8/p3`–`p5`. See `docs/decisions.md`, "A8b – faucet removed".

## Phase 2: dashboard
- Setup text corrected; the runner address is shown in full (`.addr-full`, selectable, no Copy button); Copy of the text to sign falls back to `execCommand('copy')` and then "Press Ctrl+C to copy"; project logo in header, login and favicon, PNGs served as `image/png`.
