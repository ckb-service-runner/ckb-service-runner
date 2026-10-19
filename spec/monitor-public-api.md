# CSR Network Monitor – public API (read-only)

Consumed by the CSR dashboard (CSR monitor page) through `GET /api/network`. Base URL = the effective monitor URL
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
 "check_type":"p2p_verified","attestation":{"status":"valid","expiry":1795000000},"last_check_at":1792000000}
```

- `check_type`: `p2p_verified` or `challenge_only`.
- `attestation.status`: `none`, `valid` or `expired`; `expiry` is a unix timestamp or `null`.
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
