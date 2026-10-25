# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

## Documents (csr-docs)
- First transfer: 700 CKB, stated once in README "Runner wallet, stake and registration" (539 CKB registry cell + fee + ≥61 CKB change ≈ 601, with headroom). The v3 contract is not in this repo; the figure uses the layout in `spec/registry.md`.
- `identity proven` in the dashboard: `identity_proven` from the monitor if present, else `check_type == p2p_verified`.
- `deployments/testnet.json`: `rpc` and `anyone_can_pay` removed (nothing in the runner reads them; csr-dev falls back to its constant ACP hash).
- Milestone history: index in `docs/milestones/README.md`; old URLs in `docs/spec/` and `.github/` changed to `ckb-service-runner/ckb-service-runner`; the owner name was dropped from the CRT plan.
- A7 history in specs is kept between `<!-- history -->` markers.
