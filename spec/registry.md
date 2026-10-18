# CSR registry (`csr-registry`) and the monitor config cell

Status: A3a. Reference: CSR-SPEC-v0.2 §9/§10.

## 1. Monitor config cell (also the on-chain clock)

A cell whose **type script is a Type ID script**; its type script hash is `config_type_hash`. Its lock is the
monitor's own lock, so only the monitor can update it. Data:

| offset | size | field |
|-------:|-----:|-------|
| 0 | 33 | `monitor_pubkey` (compressed secp256k1) |
| 33 | 8 | `clock_ms` (u64 LE, Unix milliseconds) |
| 41 | rest | reserved |

Scripts read it as a **cell dep**. A live cell dep proves the data is *current*. The monitor updates `clock_ms`
regularly (the weekly stat cell may do this later). On devnet `csr-dev` plays the monitor.

> **Time caveat.** On-chain time can only be *lower-bounded* (`since`, header deps). The live config cell is
> the upper bound for "now". Attestation freshness (`expiry_ms > clock_ms`) is therefore only as good as the
> monitor's clock updates: if the monitor stops updating `clock_ms`, expired attestations stay acceptable.

## 2. Registry cell

* Type script: `csr-registry`, args = `config_type_hash (32)`.
* Lock script: `csr-dual-lock`, args = `owner_lock_hash ‖ runner_lock_hash` (`float-lock.md` §3).
* Data: molecule `Registry` (`contracts/schemas/csr.mol`):

```
table Registry {
    peer_id: Bytes, service_pubkey: Byte33, float_lock_hash: Byte32, network: byte,
    attestation: Attestation, prekeys: Bytes,          // prekeys reserved for CRT
}
table Attestation { peer_id: Bytes, service_pubkey: Byte33, network: byte, expiry_ms: Uint64, signature: Signature(65) }
table AttestationBody { peer_id: Bytes, service_pubkey: Byte33, network: byte, expiry_ms: Uint64 }
```
`network`: 0 mainnet, 1 testnet, 2 devnet.

## 3. Attestation message (the monitor implements the same)

```
message   = blake2b-256( "CSR-ATTEST-v1" ‖ molecule(AttestationBody) )
signature = r (32) ‖ s (32) ‖ recovery_id (1)         // recoverable secp256k1 over `message`
```
* blake2b-256 uses the CKB personalization `ckb-default-hash`.
* `"CSR-ATTEST-v1"` is the 13 ASCII bytes without terminator.
* `molecule(AttestationBody)` is the standard molecule table encoding (header with total size and four field
  offsets, then the fields in order) of `{peer_id, service_pubkey, network, expiry_ms}`; `peer_id` is a
  molecule `Bytes` (u32 LE length ‖ bytes), `service_pubkey` 33 raw bytes, `network` 1 byte, `expiry_ms` 8 bytes LE.
* The recovered compressed public key must equal `monitor_pubkey` of the config cell.
* Verified in-script with a no_std secp256k1 implementation; cycle numbers are recorded in `docs/decisions.md`.

## 4. Rules

On **creation and on every update** (for each registry output):

1. Exactly one cell dep has type hash `config_type_hash`; its data is at least 41 bytes.
2. The attestation signature recovers to that cell's `monitor_pubkey`.
3. The attestation's `peer_id`, `service_pubkey`, `network` equal the registry data.
4. `attestation.expiry_ms > clock_ms`.
5. The output lock is `csr-dual-lock` whose `runner_lock_hash` equals the lock hash of
   `secp256k1_blake160_sighash_all` with `args = blake160(service_pubkey)`.

**One registry input and one output per group** (1:1; creation: 0 → 1, destruction: 1 → 0).

**Owner path** (an input with `owner_lock_hash` present): any field may change; rules 1–5 still apply to the
output.

**Runner path** (no owner input): only `attestation` and `prekeys` may change. `peer_id`, `service_pubkey`,
`float_lock_hash`, `network`, lock and type stay identical; output capacity ≥ input capacity; the cell must not
be destroyed.

**Destroying** (input without registry output) requires the owner path.

For *creation* there is no input to read an owner from; the output's lock args carry the owner hash and the
rules above are the only checks (anybody may register their own key; sybil resistance is the monitor's
attestation).

## 5. Error codes

| code | meaning |
|-----:|---------|
| 1–4 | ckb-std syscall errors |
| 5 | type args are not 32 bytes |
| 6 | more than one registry input or output in the group |
| 7 | registry data is not valid molecule |
| 8 | config cell dep missing or not unique |
| 9 | config cell data malformed (< 41 bytes) |
| 10 | attestation signature invalid or not by the monitor |
| 11 | attestation fields differ from registry data |
| 12 | attestation expired (`expiry_ms <= clock_ms`) |
| 13 | output lock is not `csr-dual-lock` or its runner hash does not match `service_pubkey` |
| 14 | runner path changed a forbidden field |
| 15 | runner path reduced capacity |
| 16 | destroy without owner path |

## 6. Worked examples

*Create.* Inputs `[owner wallet cell]`, outputs `[registry cell (dual-lock, type registry, data with fresh
attestation)]`, cell deps `[config cell, code deps]`.

*Runner refresh.* Inputs `[registry cell, runner key cell]`, outputs `[registry cell with new attestation
(same other fields, capacity ≥), runner key cell]`, cell deps `[config cell]`.

*Owner changes float lock.* Inputs `[registry cell, owner wallet cell]`, outputs `[registry cell with new
`float_lock_hash`]`.

*Owner destroy.* Inputs `[registry cell, owner wallet cell]`, outputs `[change]`.
