# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

## Attestation renewal (D3 A)
- The runner renews the registry attestation itself (`csr register refresh`, "Renew now", and automatically 2 days before expiry on the chain clock of the config cell, checked every 30 minutes, retried after 30 minutes on failure). A refresh needs no stake dep and no consent; only a runner-owned cell is refreshed.
- `wallet_busy`, `SWEEP_PENDING_AFTER_BLOCKS` and the dashboard password check did not exist in this codebase yet; `wallet::wallet_busy` (refresh pending or keeper submissions pending) and the constant (100 blocks) were added with this change. The dashboard route uses the `wallet_tx` lock and the `wallet_busy` check; the password check applies through the existing session/CSRF middleware.
- The last renewal error is kept in the settings key `renewal_error` and returned by `GET /api/registry`.
