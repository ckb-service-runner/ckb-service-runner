# CSR Ping Protocol v1.1

v1.1 adds the fresh check-in (§5), a normal answer for heights above the node's tip (§4), `clock_skew` and JSON error bodies.
All v1 message prefixes are unchanged (§6).

Used by the CSR Network Monitor to rank CSR nodes. A CSR node proves that it controls a service key,
is bound to a CKB node (peer ID; proven with the node's P2P key, §5b), and can read the chain at a height chosen by the monitor.
The monitor is built from this document.

## 1. Conventions

- All hex values are lowercase with a `0x` prefix.
- Numbers inside signed messages are decimal, without padding. Messages are UTF-8, fields are joined with a single `\n`, no trailing newline.
- **Signature scheme:** ECDSA over secp256k1 of `SHA-256(UTF-8 message)`, 64-byte compact `r || s`, encoded as `0x` + 128 hex chars.
- **Public keys:** compressed (33 bytes, prefix `02`/`03`), `0x` + 66 hex chars.
- **Low-S / S normalisation:** CSR always produces low-S signatures (`s <= n/2`). Before verifying a monitor signature CSR
  normalises S (`s := n - s` if `s > n/2`), so signers that produce high-S signatures (e.g. OpenSSL) are accepted.
  The monitor must accept CSR signatures (always low-S) and should normalise S likewise.
- `nonce`: `0x` + 64 lowercase hex chars (32 random bytes), unique per challenge.
- `network`: `testnet` or `mainnet`. A monitor serves one network and must reject answers for another.

## 2. Messages

| Name | Message |
|---|---|
| Challenge | `CSR-PING-CHALLENGE-v1\n{nonce}\n{height}\n{issued_at}` signed by the monitor key |
| Binding | `CSR-BINDING-v1\n{peer_id}\n{service_pubkey}\n{network}` signed by the CSR service key |
| Check-in | `CSR-CHECKIN-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}` signed by the CSR service key (`ts` = unix seconds) |
| Answer | `CSR-PING-ANSWER-v1` followed by the fields below, each on its own line, signed by the CSR service key |
| Node identity | `CSR-NODE-IDENTITY-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}` signed by the node's P2P key **and** by the CSR service key (§5b) |

Answer fields in signing order: `nonce, peer_id, service_pubkey, binding_sig, network, csr_version, node_version,
r2_state, tip_number, tip_hash, block_hash_at_height, uptime_s, ping_mode`
(i.e. `CSR-PING-ANSWER-v1\n{nonce}\n{peer_id}\n…\n{ping_mode}`).

## 3. JSON shapes

Challenge:
```json
{"nonce":"0x<64 hex>","height":900,"issued_at":1700000000,"monitor_sig":"0x<128 hex>"}
```
`height` and `issued_at` are JSON numbers (unix seconds). Unknown fields are rejected.

Answer (`answer` carries the 13 signed fields; `tip_number` and `uptime_s` are JSON numbers, the rest strings):
```json
{"answer":{"nonce":"0x…","peer_id":"Qm…","service_pubkey":"0x02…","binding_sig":"0x…","network":"testnet",
           "csr_version":"0.1.0","node_version":"0.210.0","r2_state":"Synced","tip_number":1000,
           "tip_hash":"0x…","block_hash_at_height":"0x…","uptime_s":3600,"ping_mode":"inbound"},
 "sig":"0x<128 hex>"}
```
- `peer_id` and `node_version` come from the node's `local_node_info` (`node_id`, `version`).
- `r2_state` is the real current R2 state (`Unknown, Unreachable, Degraded, Syncing, Synced`); answers are sent even when not Synced.
- `block_hash_at_height` is `get_block_hash(height)` of the node; `tip_hash` is the hash of the tip header.
  When `height` is above the node's tip the answer is still signed and sent, with `block_hash_at_height` empty (`""`):
  the monitor records "reachable, not in sync" instead of a failure.
- `ping_mode` is the configured mode (`inbound`, `outbound`, `both`).
- `binding_sig` is the Binding message signed with the service key, so one signature proves peer ID ↔ service key.

Check-in (outbound mode): `{"peer_id","service_pubkey","binding_sig","network","csr_version","ts","checkin_sig"}`;
`ts` is a JSON number (unix seconds), `checkin_sig` the signature of the Check-in message. A v1 monitor ignores `ts` and `checkin_sig`.

## 4. Inbound flow (monitor → CSR)

1. The monitor picks a height `h` at or below the chain tip (preferably a few blocks deep, so all honest nodes know it), a fresh random nonce, `issued_at = now`, and signs the Challenge message.
2. `POST http://<ping_bind>/v1/challenge` with the Challenge JSON (`Content-Type: application/json`, body ≤ 4096 bytes).
3. CSR answers `200` with the Answer JSON.
4. The monitor verifies: `sig` over the Answer message with `service_pubkey`; `binding_sig` over the Binding message with `service_pubkey`;
   `nonce` equals the challenge; `block_hash_at_height` equals the canonical hash at `h` (or is empty when `tip_number < h`, which counts as reachable but not in sync); `network` matches; `tip_number >= h`.
   The monitor learns the (`peer_id`, `service_pubkey`) pair from the binding and should check `peer_id` against the node ID it sees on the CKB P2P network.

CSR serves only `POST /v1/challenge` on a separate listener (`ranking.ping_bind`, default `0.0.0.0:8131`). It is open only when
`ranking.join = true` and `ping_mode` is `inbound` or `both`. Other paths return 404, other methods 405.

### Rejections (no answer is produced)

Every rejection has a JSON body `{"error": "<code>"}`.

| Status | `error` | Reason |
|---|---|---|
| 400 | `bad_request` | malformed JSON / unknown fields / invalid nonce format |
| 400 | `clock_skew` | `abs(now - issued_at) > 60` s; CSR logs a warning (at most once per 10 minutes) and shows it on the Network page |
| 403 | `bad_monitor_signature` | `monitor_sig` is invalid for every configured monitor key (after S normalisation) |
| 404 / 405 | – | other route / method |
| 409 | `replay` | nonce already answered within the last 10 minutes |
| 413 | `too_large` | body larger than 4096 bytes |
| 429 | `rate_limited` | more than 10 requests per minute from one client IP |
| 503 | `node_unavailable` | the node tip, node info or `get_block_hash` is unavailable |

A height above the node's tip is **not** a rejection: the answer carries an empty `block_hash_at_height`.

**Rate limit:** 10 requests per rolling 60 s per client IP, counting every request regardless of outcome; the 11th gets `429`.
A failed attempt (503) does not burn the nonce; a successful answer does, for 10 minutes.

## 5. Outbound flow (CSR → monitor)

Used by nodes that cannot accept inbound connections (NAT). Mode `outbound` or `both`, `ranking.join = true`.
Every 10 minutes (and once right after the first R2 check), to each configured monitor independently
(one failing monitor does not stop the others):

1. CSR posts the check-in JSON (§3) to `<monitor.url>/v1/checkin`. The check-in is fresh: `ts` is the current time and
   `checkin_sig` signs `CSR-CHECKIN-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}` with the service key, so a copied
   binding cannot be replayed to burn the runner's check-in slots. A v1.1 monitor accepts `ts` within ±60 s of its own
   time and one use per service key and `ts`; otherwise it refuses the check-in.
2. The monitor replies `200` `{"challenge":{…}}` (the Challenge JSON of §3), using the same rules for height, nonce and `issued_at`.
3. CSR validates the challenge exactly like an inbound one (§4 rejections; a height above the tip is answered with an empty block hash).
4. CSR posts the Answer JSON (§3) to `<monitor.url>/v1/answer`. The monitor verifies as in §4 step 4 and
   additionally checks that the nonce was issued to this `peer_id` and not yet used.
5. Failures are retried at the next interval.

The monitor URL is `https` (plain `http` only for loopback hosts). Monitors are configured as a list:

```toml
[[monitors]]
url = "https://monitor.example"
pubkey = "0x02…"        # compressed secp256k1 key of that monitor
```

The default is the network's reference monitor (one entry); the old `[monitor]` section still works as a one-entry list,
and setting both is a configuration error. The inbound listener accepts a challenge signed by any configured monitor;
the check-in goes to each monitor; attestations (§5a) are requested from the first one.

## 5a. Attestation request (`POST /v1/attest`)

`csr register attest --stake 0x<tx>:<index> --out FILE` asks the monitor for the registration attestation
(`spec/registry.md` §3). Request body, exactly these fields:

```json
{"peer_id":"Qm…","service_pubkey":"0x02…","binding_sig":"0x…","network":"testnet",
 "stake":"0x<64 hex>:<index>"}
```

- `peer_id`, `service_pubkey`, `binding_sig`, `network` as in the check-in (§3); `binding_sig` is the Binding message.
- `stake` is the out point of the Nervos DAO deposit the registration will reference (registry field `stake`),
  `0x` + 64 lower-case hex digits of the tx hash, `:`, the output index in decimal. CSR refuses a malformed value
  before sending anything.

Replies: `200` with the attestation JSON (`{peer_id, service_pubkey, network, network_id, expiry_ms, signature}`);
errors are JSON `{"error": "<code>"}`, optionally with `"reason"`. Errors and replies are listed in the one table in
`spec/monitor-public-api.md` (`POST /v1/attest`). CSR prints the status, `error`, `reason` and a hint
(e.g. `monitor replied 403 stake_invalid: too_small (the deposit must be at least 10,000 CKB)`) and exits non-zero.

## 5b. Node identity (`POST /v1/identity`, D6 A)

The Binding (§2) is signed by the service key only, so it is a claim. The node-identity proof is signed by
the CKB node's P2P secret key as well, so only someone holding the node's key can bind a service key to it.

- **Peer ID.** A CKB peer ID is `base58(0x12 ‖ 0x20 ‖ SHA-256(p2p_pubkey))` (Bitcoin base58 alphabet;
  multihash SHA-256, as tentacle's `PeerId::from_public_key`), where `p2p_pubkey` is the 33-byte compressed
  secp256k1 public key.
- **Message.** `CSR-NODE-IDENTITY-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}`; `ts` is unix seconds
  (decimal). Both signatures use the scheme of §1 over the same message.
- **When.** CSR reads the key only if the operator sets `node.p2p_key_file`, and sends a proof only if the
  key derives the `node_id` the node reports (`local_node_info`). It posts the proof when `ranking.join` is
  true: at start, every 24 h, and when the node answers again after being unreachable (node restart).
  Failures (network, 5xx) are retried after 10 minutes.

Request body, exactly these fields:

```json
{"peer_id":"Qm…","service_pubkey":"0x02…","network":"testnet","ts":1792281600,
 "p2p_pubkey":"0x02…","p2p_sig":"0x<128 hex>","service_sig":"0x<128 hex>"}
```

The monitor verifies: `peer_id` is derived from `p2p_pubkey`; `p2p_sig` with `p2p_pubkey` and `service_sig`
with `service_pubkey`, both over the message (after S normalisation); `network` is its own; `ts` is fresh;
and `peer_id` is the node ID it sees for the node (monitor spec `node-identity-monitor`).

| Status | Body | Meaning |
|---|---|---|
| 200 | `{"ok": true}` | proof accepted |
| 400 | `{"error": "bad_request"}` | malformed JSON, unknown or missing fields, wrong network, `peer_id` not derived from `p2p_pubkey` |
| 400 | `{"error": "stale"}` | `ts` too far from the monitor's clock |
| 401 | `{"error": "bad_signature"}` | `p2p_sig` or `service_sig` invalid |
| 403 | `{"error": "peer_mismatch"}` | the node the monitor sees under this address/connection has another peer ID |

A monitor without this endpoint answers 404; CSR treats that as "not supported yet" (debug log, retried in
24 h). The last result per monitor is shown on the dashboard's Network page.

### `p2p_address` (D12 A)

`GET /v1/monitor` carries the P2P address of the monitor's own CKB node:

```json
{"p2p_address": "/ip4/203.0.113.7/tcp/8115/p2p/Qm…", "…": "…"}
```

A multiaddr `/ip4|ip6|dns4|dns6/<host>/tcp/<port>/p2p/<peer_id>`. CSR splits it into the peer ID and the
address without `/p2p/…` and, at start and every 6 h (`ranking.join` true), calls the node RPC
`add_node(peer_id, address)` unless `get_peers` already lists that peer, so that a runner behind NAT
connects out to the monitor's node and the monitor can check it among its own peers. A missing or
unparsable field is ignored. CSR never calls `remove_node` or other Net methods; "method not found" means the
node's Net RPC module is off (CSR warns once).

## 6. Versioning

The `-v1` suffix in every message prefix is the message version; protocol v1.1 adds the Check-in message and keeps all v1 prefixes. A change of message layout requires a new prefix; signatures
over different prefixes can never be confused.
