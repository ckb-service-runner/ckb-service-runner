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

- `rank` is `null` for an unranked node (provisional, flagged, or known only from its registry cell); `of` is the
  number of ranked nodes. `"provisional"` (boolean) is added; older monitors send none, CSR treats that as `false`.
- Unranked example: `{"peer_id":"Qm…","rank":null,"of":42,"provisional":true,"availability":{"d7":null,"d30":null,"d365":null},"check_type":"none","attestation":{"status":"none","expiry":null},"last_check_at":0,"stake":{"status":"none","capacity_ckb":null,"outpoint":null}}`
- `availability` values (`d7`, `d30`, `d365`) may be `null`.
- `check_type`: `p2p_verified` or `challenge_only`; `none` for a node known only from its registry cell.
- `stake` and `attestation` are always present, also for unranked nodes.
- `attestation.status`: `none`, `valid` or `expired`; `expiry` is a unix timestamp or `null`.
- `stake` (optional; older monitors omit it, CSR then shows nothing): the registration's referenced Nervos DAO
  deposit (`spec/registry.md`, stake). `status` is `deposited` (live DAO deposit, enough CKB), `withdrawn`
  (spent or in the withdrawing phase), `invalid` (not a deposit, too small or not owned by the registry owner),
  `duplicate` (an older registration references the same deposit) or `none` (no stake in the registry cell).
  `capacity_ckb` is a number or `null`; `outpoint` is `0x<64 hex>:<index>` or `null`.
- 404 `{"error": "unknown_node"}` is used only for an unknown peer.

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

Replies of `POST /v1/attest` (one table, from the monitor's code; the runner explains each with `explain_error`).
Errors are `{"error": "<code>"}`, some with `"reason"`:

| Status | `error` | `reason` | Meaning |
|---|---|---|---|
| 400 | `bad_request` | – | malformed body or field (also a malformed `stake`) |
| 400 | `wrong_network` | – | `network` is not the network this monitor serves |
| 401 | `bad_signature` | – | `binding_sig` does not verify for `peer_id`, `service_pubkey` and `network` |
| 403 | `not_eligible` | – | the node has no successful checks in the last 24 hours |
| 403 | `stake_invalid` | `spent` | the stake cell is already spent |
| 403 | `stake_invalid` | `not_found` | the stake cell is unknown |
| 403 | `stake_invalid` | `not_dao` | the cell is not a Nervos DAO cell |
| 403 | `stake_invalid` | `not_deposit` | a Nervos DAO cell that is not a deposit (e.g. withdrawing) |
| 403 | `stake_invalid` | `too_small` | capacity below `MIN_STAKE` (10,000 CKB) |
| 403 | `stake_in_use` | – | another registration already references this deposit (the oldest registration wins) |
| 403 | `stake_mismatch` | – | the stake is not the stake of the registry cell |
| 429 | `rate_limited` | – | too many requests |
| 503 | `chain_unavailable` | – | the monitor cannot read the chain; try again later |
| 503 | `clock_stale` | – | the monitor's chain view is behind; try again later |
| 503 | `unavailable` | – | the monitor is temporarily unavailable |

Clients show `error` and `reason` as given and ignore unknown `reason` values. With several monitors
(`[[monitors]]`), the runner asks the first one.
