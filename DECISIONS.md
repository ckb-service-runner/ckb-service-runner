# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

## Dashboard security, phase 1
- See `docs/decisions.md` ("Dashboard security, phase 1"): password required for money/registration/service/settings writes, Host/Origin checks, login limiter, split session pools, Secure cookie behind HTTPS.
- Dashboard notice: a banner on the Wallet and Settings pages (registration lives on the Wallet page) while no password is set, and on any `password_required` refusal, linking to the Settings password note.
See docs/decisions.md, "Sandbox storage, phase 1".
