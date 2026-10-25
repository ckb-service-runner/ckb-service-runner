# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

Contracts v3, phase 1: see `docs/decisions.md`, section "Contracts v3 – one redeploy before mainnet (D20–D24)", subsection "Contracts v3 Phase 1 – contracts" (registry args = config Type ID with the config type hash computed from the embedded `csr-config-type` code hash; `csr-config-type` referenced with hash type data2 because `ckb_std::type_id` brings A-extension atomics; how the shared vector is checked "inside the contract"; job lock signature failures map to error 10; order of the same-index checks; the test-only job-lock variant build; the test crate's fallback `build.rs`).
