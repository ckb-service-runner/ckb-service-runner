# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

Contracts v3, phase 1: see `docs/decisions.md`, section "Contracts v3 – one redeploy before mainnet (D20–D24)", subsection "Contracts v3 Phase 1 – contracts" (registry args = config Type ID with the config type hash computed from the embedded `csr-config-type` code hash; `csr-config-type` referenced with hash type data2 because `ckb_std::type_id` brings A-extension atomics; how the shared vector is checked "inside the contract"; job lock signature failures map to error 10; order of the same-index checks; the test-only job-lock variant build; the test crate's fallback `build.rs`).

Contracts v3, phase 2: see `docs/decisions.md`, subsection "Contracts v3 Phase 2 – runner and devtools" (v2/v3 deployments side by side with `register create` refused on v2; no separate refresh command; 64-byte runner-owned args still recognised; format of the monitor's v2 reply; checks before broadcasting; endpoint validation; `csr-dev` `--keep-config` and clock rules).

Contracts v3, phase 2 (retry): the previous run failed only because `acceptance/aiworker-contracts-v3/registry_v3_tests.rs` had been deleted; the runner/devtools work was kept as is, and the files under `acceptance/aiworker-contracts-v3/` are left untouched (`chain.rs` only includes the test module).
