# CSR Network Monitor – public API

The `GET` endpoints are read-only and consumed by the CSR dashboard (CSR monitor page) through `GET /api/network`.
`POST /v1/attest` (end of this file) is used by `csr register attest`. Base URL = the effective monitor URL
(`monitor.url` or the network default). `https` only, `http` only for loopback hosts. Timeout 10 s. No authentication.
Availability values are fractions 0..1. Clients ignore unknown extra fields.

## `GET /v1/network`

```json
{"nodes_total":42,"nodes_reachable":30,"countries":[{"code":"DE","count":12}],"updated_at":1792000000}
```

## `GET /v1/nodes/{peer_id}`

`peer_id` is URL-encoded. 200:

```json
{"peer_id":"Qm…","rank":3,"of":42,"availability":{"d7":0.99,"d30":0.98,"d365":0.95},
 "check_type":"p2p_verified","attestation":{"status":"valid","expiry":1795000000},"last_check_at":1792000000,
 "stake":{"status":"deposited","capacity_ckb":10000,"outpoint":"0x5a5a…5a5a:3"}}
```

- `check_type`: `p2p_verified` (passed the daily full-node check; the node's identity is proven), `peer_connected` (the node is a peer of the monitor's node), `challenge_only` (older monitors) or `none`. An optional `identity_proven` boolean overrides the dashboard's inference from `check_type`.
- `attestation.status`: `none`, `valid` or `expired`; `expiry` is a unix timestamp or `null`.
- `stake` (optional; older monitors omit it, CSR then shows nothing): the registration's referenced Nervos DAO
  deposit (`spec/registry.md`, stake). `status` is `deposited` (live DAO deposit, enough CKB), `withdrawn`
  (spent or in the withdrawing phase), `invalid` (not a deposit, too small or not owned by the registry owner),
  `duplicate` (an older registration references the same deposit) or `none` (no stake in the registry cell).
  `capacity_ckb` is a number or `null`; `outpoint` is `0x<64 hex>:<index>` or `null`.
- 404 when the node is not ranked.

## `GET /v1/locations`

```json
{"locations":[{"lat":53,"lon":13,"count":2}],"located":6,"unlocated":2,"grid_deg":2,"updated_at":1792324800}
```

- `lat`/`lon` are the centres of 2° × 2° cells; the monitor never publishes exact positions. `count` = runners in the cell.
- `unlocated` counts runners with unknown location; they are not drawn.
- Older monitors answer 404: CSR then shows a map without dots.

`GET /api/network` adds to its `network` object: `locations` (the list above), `unlocated` and
`locations_available` (false when `/v1/locations` is missing or failed). A failing `/v1/locations` never makes the
network data an error or stale. Coordinates are display-only and are never logged or stored.

CSR caches these replies for 5 minutes and serves stale data (flagged) if a refresh fails.

## `POST /v1/attest`

Requests the registration attestation (`spec/registry.md` §3); the request is described in
`spec/ping-protocol.md` §5a:

```json
{"peer_id":"Qm…","service_pubkey":"0x02…","binding_sig":"0x…","network":"testnet",
 "stake":"0x5a5a…5a5a:3"}
```

`stake` is the out point (`0x<64 hex>:<index>`, lower case) of the Nervos DAO deposit the registry cell will name in
its `stake` field. The monitor checks it with the same rules as `csr-registry` (`spec/registry.md`, stake rules 2–4)
plus the rules a contract cannot check: the deposit is live, and no other registration already uses it.

200: the attestation `{"peer_id","service_pubkey","network","network_id","expiry_ms","signature"}`.

Errors (`{"error": "<code>"}`, some with `"reason"`):

| Status | `error` | `reason` | Meaning |
|---|---|---|---|
| 400 | `bad_request` | – | malformed body or field (also a malformed `stake`) |
| 403 | `not_eligible` | – | the node has no successful checks in the last 24 hours |
| 403 | `stake_invalid` | `not_found` | the stake cell is unknown or already spent |
| 403 | `stake_invalid` | `not_dao_deposit` | not a Nervos DAO deposit (type script or data; a withdrawing cell is refused) |
| 403 | `stake_invalid` | `too_small` | capacity below `MIN_STAKE` (10,000 CKB) |
| 403 | `stake_invalid` | `wrong_owner` | the deposit's lock is not the registration's owner lock (checked when the monitor knows the owner, e.g. on renewal) |
| 409 | `stake_in_use` | – | another registration already references this deposit (the oldest registration wins) |

Clients show `error` and `reason` as given and ignore unknown `reason` values.
