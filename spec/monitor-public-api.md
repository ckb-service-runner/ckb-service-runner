# CSR Network Monitor – public API (read-only)

Consumed by the CSR dashboard (Network page) through `GET /api/network`. Base URL = the effective monitor URL
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

CSR caches both replies for 5 minutes and serves stale data (flagged) if a refresh fails.
