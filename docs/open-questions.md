# Open questions

- Real monitor URL and pubkey per network: placeholders remain in `config/networks/*.toml` where no monitor runs.
- Neuron RPC port and indexer: does the embedded Neuron node expose `get_indexer_tip`? `node_type = "neuron"` is accepted but untested.
- Monitor reply format: does the real `/v1/answer` reply carry data beyond HTTP 200 (CSR treats 200 as accepted)?
- Outbound check-in cadence on failure: retry sooner than 10 min?
- `get_peers` / `sync_state` field names were taken from the ckb 0.210 RPC docs; verify against a real node.
- Offline-signing file format: the `ckb-cli` tx JSON is written by `csr-dev`; whether Neuron imports it is unverified.
- Audit decisions still open: when the mainnet gate opens per release (D4 A pauses only before a run starts, so a
  long run is never interrupted: is that acceptable on mainnet?), and whether the monitor should expose
  `identity_proven` directly instead of the dashboard inferring it from `check_type`.
- CRT: prekey support (`prekey_decrypt`, `random_bytes`) is reserved and returns 2.
