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
* `recovery_id` is 0–3; `s` must be low (≤ n/2, as produced by libsecp256k1 / `ckb-crypto`); `r`, `s` big endian.
* The recovered compressed public key must equal `monitor_pubkey` of the config cell.
* Verified in-script with the pure-Rust `k256` 0.13 crate (`VerifyingKey::recover_from_prehash`); the message is
  hashed with `blake2b-ref`. See §7 for cycles.

Byte layout of the hashed input for `peer_id` of length *p*:

| offset | size | content |
|-------:|-----:|---------|
| 0 | 13 | `CSR-ATTEST-v1` |
| 13 | 4 | total size of the body = 20 + 4 + p + 33 + 1 + 8 (u32 LE) |
| 17 | 16 | field offsets 20, 24 + p, 57 + p, 58 + p (u32 LE each) |
| 33 | 4 + p | `peer_id` (u32 LE length ‖ bytes) |
| 37 + p | 33 | `service_pubkey` |
| 70 + p | 1 | `network` |
| 71 + p | 8 | `expiry_ms` (u64 LE) |

## 4. Rules

On **creation and on every update** (for each registry output):

1. Exactly one cell dep has type hash `config_type_hash`; its data is at least 41 bytes.
2. The attestation signature recovers to that cell's `monitor_pubkey`.
3. The attestation's `peer_id`, `service_pubkey`, `network` equal the registry data.
4. `attestation.expiry_ms > clock_ms`.
5. The output lock is `csr-dual-lock` whose `runner_lock_hash` equals the lock hash of
   `secp256k1_blake160_sighash_all` with `args = blake160(service_pubkey)`.
   * "Is `csr-dual-lock`": `code_hash` = data hash of the `csr-dual-lock` binary of the same build (embedded into
     `csr-registry` at build time by `csr-registry/build.rs`), `hash_type` `data1` or `data2`, args 64 bytes.
   * Accepted forms of the sighash lock: `code_hash 0x9bd7e06f…a3cce8` with `hash_type type` (the genesis
     deployment on mainnet, testnet and dev chains), or the binary's data hash `0x709f3fda…e08649`
     (ckb-system-scripts 0.5.4) with `hash_type data`, `data1` or `data2`. All four run the same code.

6. **Runner key present** (creation, and owner-path updates that change `service_pubkey` or the output's runner
   lock hash): the transaction has an input whose lock hash equals the **output's** `runner_lock_hash`
   (dual-lock `args[32..64]`), i.e. the node's runner key cell. This proves possession of the service key
   (error 17). It is checked after rules 1–5.

Data must be a complete molecule `Registry` with exactly the schema's field counts (no extra fields), and
`network` ≤ 2; otherwise error 7.

**One registry input and one output per group** (1:1; creation: 0 → 1, destruction: 1 → 0).

**Owner path** (an input with `owner_lock_hash` present): any field may change; rules 1–5 still apply to the
output. If `service_pubkey` or the runner lock hash changes, rule 6 requires the runner key cell of the
**new** runner lock hash (the old service key does not authorise a new one).

**Runner path** (no owner input): only `attestation` and `prekeys` may change. `peer_id`, `service_pubkey`,
`float_lock_hash`, `network`, lock and type stay identical; output capacity ≥ input capacity; the cell must not
be destroyed.

**Destroying** (input without registry output) requires the owner path.

For *creation* there is no input to read an owner from; the output's lock args carry the owner hash. Rules 1–6
apply: anybody may register **their own** key (sybil resistance is the monitor's attestation), but only the
holder of the service key can create an entry for it (rule 6).

Why rule 6 (audit finding **F1**): attestations are public on chain. Without it anyone could copy another
node's attestation and create a second registry cell with that node's `peer_id` and `service_pubkey` but their
own owner, `float_lock_hash` and `prekeys`, then collect tips (or publish fake prekeys) under that identity.

Note (**F8**): the same key holder can still create several registry cells for one service key. Off-chain
consumers (CSR, monitor, CRT) must pick the one with the **latest** attestation (`expiry_ms`) and ignore the
others.

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
| 17 | no input with the output's runner lock hash (creation, or owner path changing the service key) |

## 6. Worked examples

*Create.* Inputs `[owner wallet cell, runner key cell]`, outputs `[registry cell (dual-lock, type registry,
data with fresh attestation), change, runner key cell]`, cell deps `[config cell, code deps]`. Both inputs are
signed (owner wallet by its lock, runner key cell with the service key). The runner key cell may also be the
only input if it pays for the registry cell.

*Rejected creation:* owner wallet only (17, `registry_create_without_runner_key_rejected`); attacker as owner
with the victim's `peer_id`, `service_pubkey` and copied attestation, the attacker's float and the attacker's
own key cell (17, `registry_create_with_copied_attestation_rejected`).

*Runner refresh.* Inputs `[registry cell, runner key cell]`, outputs `[registry cell with new attestation
(same other fields, capacity ≥), runner key cell]`, cell deps `[config cell]`.

*Owner changes float lock.* Inputs `[registry cell, owner wallet cell]`, outputs `[registry cell with new
`float_lock_hash`]`.

*Owner rotates the service key.* Inputs `[registry cell, owner wallet cell, new runner key cell]`, outputs
`[registry cell with new `service_pubkey`, a fresh attestation for it and dual lock `owner ‖ new runner hash`,
change, new runner key cell]` (`registry_owner_changes_service_pubkey_with_new_runner_key_ok`). Without the new
key cell, or with only the old one, → 17.

*Owner destroy.* Inputs `[registry cell, owner wallet cell]`, outputs `[change]`.

Owner presence on update/destroy is taken from the **input** cell's lock args (`owner_lock_hash` = first 32
bytes of its dual-lock args). On the owner path the output may carry a different dual lock (e.g. a new owner),
as long as rule 5 holds.

## 7. Use by the job lock

`csr-job-lock` (args = hash of this registry type script) reads a registry cell as a **cell dep** and takes
its `float_lock_hash` as the float that must receive the tip (`job-cell-standard.md`). It does not re-check
the attestation: a registry cell whose `expiry_ms` has passed (relative to the monitor's `clock_ms`) still
counts as registered for job runs until its runner refreshes it via the runner path. Off-chain clients
(the job creator, the monitor) should prefer runners with a fresh attestation. Since `float_lock_hash` can only
be changed by the owner path, a compromised runner key cannot redirect tips.

## 8. Cycles

Measured with ckb-testtool (`registry_create_cycles_of_type_script`, `registry_create_with_valid_attestation_ok`,
`registry_runner_refreshes_attestation_ok`; peer ID of 40 bytes, release build `opt-level = "s"`):

| transaction | cycles |
|-------------|-------:|
| registry creation, `csr-registry` type script alone (tx minus a key-cell-only baseline) | 7,951,415 |
| registry creation, whole tx (type script + two system-lock inputs: wallet, runner key) | 11,344,015 |
| runner refresh, whole tx (type script + dual lock + system lock) | 9,660,915 |

Almost all of it is the secp256k1 public-key recovery in `k256`; the binary is ~89 KB.
