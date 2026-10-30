# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

## Dashboard security, phase 1
- See `docs/decisions.md` ("Dashboard security, phase 1"): password required for money/registration/service/settings writes, Host/Origin checks, login limiter, split session pools, Secure cookie behind HTTPS.
- Dashboard notice: a banner on the Wallet and Settings pages (registration lives on the Wallet page) while no password is set, and on any `password_required` refusal, linking to the Settings password note.
See docs/decisions.md, "Sandbox storage, phase 1".
## Attestation renewal (D3 A)
- The runner renews the registry attestation itself (`csr register refresh`, "Renew now", and automatically 2 days before expiry on the chain clock of the config cell, checked every 30 minutes, retried after 30 minutes on failure). A refresh needs no stake dep and no consent; only a runner-owned cell is refreshed.
- `wallet_busy`, `SWEEP_PENDING_AFTER_BLOCKS` and the dashboard password check did not exist in this codebase yet; `wallet::wallet_busy` (refresh pending or keeper submissions pending) and the constant (100 blocks) were added with this change. The dashboard route uses the `wallet_tx` lock and the `wallet_busy` check; the password check applies through the existing session/CSRF middleware.
- The last renewal error is kept in the settings key `renewal_error` and returned by `GET /api/registry`.
## ping-v1-1-runner
- Empty `url`/`pubkey` of a `[[monitors]]` entry take the network default, like the old `[monitor]` fields.
- An entry with an unusable key is skipped with a warning on the inbound side; `attest` uses the first entry as given.
- The 4096-byte body limit is checked in the handler (axum limit raised to 64 KiB) so `too_large` has a JSON body.
- The Network page shows a clock-skew warning via `ping.clock_skew_at` of `GET /api/network`.
- The monitor repository's spec and `docs/PROTOCOL.md` are outside this directory and not changed here.
## Node identity – runner, Phase 1

See `docs/decisions.md`, section "Node identity – runner" (D6 A, D12 A and the "Phase 1 implementation notes"): single configured monitor (no `Config::monitors()` here), own base58 instead of `bs58` (offline build), erase-on-drop instead of a `zeroize` setup that does not exist here, the extra `unknown` identity state, `add_node` gated on `ranking.join` only, 10-minute retries after failures.
csr-update-check phase 1: see `docs/decisions.md`, section "csr-update-check – the mainnet gate is switched on by a release (D5 B)".
Contracts v3, phase 1: see `docs/decisions.md`, section "Contracts v3 – one redeploy before mainnet (D20–D24)", subsection "Contracts v3 Phase 1 – contracts" (registry args = config Type ID with the config type hash computed from the embedded `csr-config-type` code hash; `csr-config-type` referenced with hash type data2 because `ckb_std::type_id` brings A-extension atomics; how the shared vector is checked "inside the contract"; job lock signature failures map to error 10; order of the same-index checks; the test-only job-lock variant build; the test crate's fallback `build.rs`).

Contracts v3, phase 2: see `docs/decisions.md`, subsection "Contracts v3 Phase 2 – runner and devtools" (v2/v3 deployments side by side with `register create` refused on v2; no separate refresh command; 64-byte runner-owned args still recognised; format of the monitor's v2 reply; checks before broadcasting; endpoint validation; `csr-dev` `--keep-config` and clock rules).

Contracts v3, phase 2 (retry): the previous run failed only because `acceptance/aiworker-contracts-v3/registry_v3_tests.rs` had been deleted; the runner/devtools work was kept as is, and the files under `acceptance/aiworker-contracts-v3/` are left untouched (`chain.rs` only includes the test module).
