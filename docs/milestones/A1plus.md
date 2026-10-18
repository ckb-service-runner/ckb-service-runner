# Milestone A1+ – Network page

## Built
- Monitor public-API client with a 5-minute cache and `GET /api/network` (phase 1).
- Dashboard page **Network** (`#/network`): nodes worldwide and reachable, per-country bar list (compact SVG, no map),
  own rank (`rank / of`), availability 7/30/365 days, check type, attestation status and expiry, ping mode and last successful check,
  a visible "stale" hint, and "Join the ranking to see stats" when `ranking.join = false` (the monitor is not contacted).
- **Overview** ranking card shows rank and 30-day availability.
- Assets stay within the A1 rules (≤ 200 KB, no external URLs, no inline scripts/handlers/style attributes).

## Demo
1. Start a mock monitor serving `/v1/network` and `/v1/nodes/{peer_id}` (see `acceptance/a1p/netmon.mjs`).
2. Set `ranking.join = true` and `monitor.url = "http://127.0.0.1:<port>"`, run `csr run`.
3. Open `http://127.0.0.1:8130/#/network` and the Overview page.
4. Stop the mock: after the cache expires the page shows the last data marked stale. Set `join = false`: the join hint appears.

## Monitor API contract
See `spec/monitor-public-api.md`.
