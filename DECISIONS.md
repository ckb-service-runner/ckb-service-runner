# Decisions

See `docs/decisions.md`, section "A8b – faucet removed". Additionally: the stale hash in the unit test `chain::tests::embedded_testnet_deployment_parses` was updated to the registry code hash of `deployments/testnet.json` (A8 redeployment).

## ping-v1-1-runner
- Empty `url`/`pubkey` of a `[[monitors]]` entry take the network default, like the old `[monitor]` fields.
- An entry with an unusable key is skipped with a warning on the inbound side; `attest` uses the first entry as given.
- The 4096-byte body limit is checked in the handler (axum limit raised to 64 KiB) so `too_large` has a JSON body.
- The Network page shows a clock-skew warning via `ping.clock_skew_at` of `GET /api/network`.
- The monitor repository's spec and `docs/PROTOCOL.md` are outside this directory and not changed here.
