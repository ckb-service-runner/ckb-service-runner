# Open questions

- Real monitor URL and pubkey per network (placeholders in config/networks/*.toml).
- docs/spec/ was missing in phase 1; please add it.
- Neuron RPC port and indexer: which port does the embedded Neuron node use, and does it expose `get_indexer_tip`? `node_type = "neuron"` is accepted but untested.
- P2P-key binding: the Binding message is signed by the service key only. Should the node's P2P key also sign it, so the monitor can verify the peer ID ownership cryptographically instead of observing it on the P2P network?
- Monitor reply format: does the real `/v1/answer` reply carry data beyond HTTP 200 (CSR treats 200 as accepted)?
- Outbound check-in cadence on failure: retry sooner than 10 min?
- `get_peers` / `sync_state` field names were taken from the ckb 0.210 RPC docs; verify against a real node.
