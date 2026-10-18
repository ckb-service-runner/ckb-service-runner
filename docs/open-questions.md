# Open questions

- Real monitor URL and pubkey per network (placeholders in config/networks/*.toml).
- docs/spec/ was missing in phase 1; please add it.
- Neuron RPC port and indexer: which port does the embedded Neuron node use, and does it expose `get_indexer_tip`? `node_type = "neuron"` is accepted but untested.
- P2P-key binding: the Binding message is signed by the service key only. Should the node's P2P key also sign it, so the monitor can verify the peer ID ownership cryptographically instead of observing it on the P2P network?
- Monitor reply format: does the real `/v1/answer` reply carry data beyond HTTP 200 (CSR treats 200 as accepted)?
- Outbound check-in cadence on failure: retry sooner than 10 min?
- `get_peers` / `sync_state` field names were taken from the ckb 0.210 RPC docs; verify against a real node.
- Transaction file format for offline signing: `csr-dev` and `csr` write the `ckb-cli` tx JSON (`transaction`, `multisig_configs`, `signatures`). Whether Neuron's offline-signing import accepts exactly this (Neuron uses its own export wrapper around the same `transaction` object) is unverified – no Neuron available here. A second export format is not added yet.

- Transaction file format for Neuron offline signing: CSR writes/reads the `ckb-cli` "tx" JSON (`transaction`, `multisig_configs`, `signatures`). Whether Neuron's offline signing accepts exactly this file could not be verified here (no Neuron available); a Neuron-specific export is not implemented.

- Windows and macOS builds are out of scope for this project (A4 ships Linux x86_64 only).
