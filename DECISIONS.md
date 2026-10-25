# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

## Node identity – runner, Phase 1

See `docs/decisions.md`, section "Node identity – runner" (D6 A, D12 A and the "Phase 1 implementation notes"): single configured monitor (no `Config::monitors()` here), own base58 instead of `bs58` (offline build), erase-on-drop instead of a `zeroize` setup that does not exist here, the extra `unknown` identity state, `add_node` gated on `ranking.join` only, 10-minute retries after failures.
