# CSR registry (`csr-registry`) and the monitor config cell

Status: contracts v3 (Registry v3, `CSR-ATTEST-v2`, `csr-config-type`; decisions D20 A–D24 A). Earlier: A7
(Registry v2: payout cell, DAO stake; decision D-A7-1), A8 (stake consent). Reference: CSR-SPEC-v0.2 §9/§10.

## 1. Monitor config cell (also the on-chain clock)

A cell whose **type script is `csr-config-type`** (`contracts/csr-config-type`, decision D24 A) with
`args = config_id (32)`, the cell's Type ID. Its lock is the monitor's own lock, so only the monitor can update
it. Data:

| offset | size | field |
|-------:|-----:|-------|
| 0 | 33 | `monitor_pubkey` (compressed secp256k1) |
| 33 | 8 | `clock_ms` (u64 LE, Unix milliseconds) |
| 41 | rest | reserved |

Scripts read it as a **cell dep**. A live cell dep proves the data is *current*. The monitor updates `clock_ms`
regularly. On devnet `csr-dev` plays the monitor.

**`csr-config-type` rules** (F-CON-006):

1. `args` are exactly 32 bytes (error 5).
2. **Type ID** (`ckb_std::type_id::check_type_id(0, 32)`): at most one input and one output carry this type
   script, and a new cell's `args` are `blake2b-256(first input of the transaction ‖ output index u64 LE)`
   (error 6). So there is only ever **one** config cell per `config_id`.
3. Data is at least 41 bytes, on creation and on every update (error 7).
4. Update: `monitor_pubkey` (bytes 0–33) is unchanged (error 8) and `clock_ms` is **not smaller** than in the
   input – it may grow or stay (error 9). The reserved bytes may change.
5. Destroying the cell (an input without output) is refused (error 10); a monitor key change means a new
   config cell and a new registry deployment.

`ckb_std::type_id` pulls in `ckb-types` (the `bytes` crate, whose atomics are RISC-V A instructions), so the
contract is referenced with **hash type `data2`** (VM version 2, which has the A extension). `csr-registry`
and `csr-job-lock` find the config cell by its **type hash**, computed from the `csr-config-type` code hash
(the data hash of the binary of the same build, embedded at build time by their `build.rs`), hash type
`data2` and `args = config_id` (their own `config_id`: the registry's type args). A Type ID cell of another
script, or `csr-config-type` with another hash type, is no config cell (error 8).

| code | meaning (`csr-config-type`) |
|-----:|---------|
| 1–4 | ckb-std syscall errors |
| 5 | type args are not 32 bytes |
| 6 | Type ID violated: more than one cell with this type in inputs or outputs, or wrong id on creation |
| 7 | data shorter than 41 bytes |
| 8 | `monitor_pubkey` changed |
| 9 | `clock_ms` went backwards |
| 10 | the config cell is destroyed |

> **Time caveat.** On-chain time can only be *lower-bounded* (`since`, header deps). The live config cell is
> the upper bound for "now". Attestation freshness (`expiry_ms > clock_ms`) is therefore only as good as the
> monitor's clock updates. Since D24 A the clock cannot go backwards, and an attestation may live at most
> **8 days** beyond the clock (`expiry_ms − clock_ms ≤ 8 days`, §4 rule 4), so a monitor key cannot issue
> attestations that outlive a stalled clock by more than that.

## 2. Registry cell

* Type script: `csr-registry`, args = `config_id (32)` (the `args` of the config cell's `csr-config-type`).
* Lock script: `csr-dual-lock`, args = `owner_lock_hash ‖ runner_lock_hash` (64 bytes) or, for a runner-owned
  cell, `runner_lock_hash` alone (32 bytes; owner = runner) (`dual-lock.md`).
* Data: molecule **`RegistryV3`** (`contracts/schemas/csr.mol`, decision D20 A):

```
struct OutPoint {
    tx_hash: Byte32,
    index:   Uint32,
}
table RegistryV3 {
    version:          byte,         // 3
    peer_id:          Bytes,
    service_pubkey:   Byte33,
    payout_lock_hash: Byte32,       // lock hash of the payout cell (since A8 the runner wallet, §9)
    network:          byte,         // 0 mainnet, 1 testnet, 2 devnet
    stake:            OutPoint,     // the operator's Nervos DAO deposit cell (§4.1)
    expiry_ms:        Uint64,       // attestation expiry (monitor clock, ms)
    attestation_sig:  Signature,    // 65 bytes r ‖ s ‖ recid, by the monitor (§3)
    endpoint:         Bytes,        // UTF-8 "https://host:port" or "http://host:port", may be empty, ≤ 128 bytes
    prekeys:          Bytes,        // reserved for CRT
}
table AttestationBodyV2 {           // signed (§3), not stored
    peer_id: Bytes, service_pubkey: Byte33, network: byte, expiry_ms: Uint64,
    stake: OutPoint, registry_type_hash: Byte32,
}
```
`OutPoint` has the layout of the CKB `OutPoint` (`tx_hash` 32 bytes, `index` u32 LE), so `stake` is
byte-for-byte the out point used in a `CellDep`.

**v3 only.** The node ID (`peer_id`, `service_pubkey`, `network`) is stored once: the attestation is no
longer a nested copy, the monitor signs the cell's own fields. Both scripts accept exactly the 10-field table
with `version = 3`: `csr-registry` rejects v1/v2 data and any other version with error 7, `csr-job-lock` rejects
such a registry dep with error 10. (The runner keeps a v2 parser to read old cells until the testnet migration.)

**`endpoint`** (F-MON-014): the runner's public address so that the monitor can call it directly; empty, or
starting with `https://` or `http://`, valid UTF-8, at most **128 bytes**; otherwise error 7. The contract does not
check the host or port.

**Size** (D20 A): with a 48-byte peer ID, empty endpoint and prekeys the data is **280 bytes** (v2: 373). With
32-byte dual-lock args for a runner-owned cell (instead of `runner ‖ runner`) a registration needs about
**125 CKB** less (93 + 32 bytes).

**Payout cell.** `payout_lock_hash` is the lock hash of the cell that receives the tips: since A8 the runner
wallet (§9); an operator may also use an anyone-can-pay cell. CSR never holds operator funds.

**Shared vector** (`acceptance/aiworker-contracts-v3/registry-v3.json`, written with the monitor's encoder and
used by the contracts, the runner and `monitor-v3`; test `registry_v3_data_matches_vector`): monitor secret
`0x…51`, monitor pubkey `0x027754b4…88522c`, peer id `QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB`,
service pubkey `0x02466d7f…ae3f27`, payout lock hash `0x77…77`, network 1, stake `0x5a…5a:3`, expiry_ms
1800000000000, registry type hash `0x4e…4e`. `AttestationBodyV2` is 190 bytes, the message
`0xc1993843cde1045f980488376eae6ab8240aad8588c7312a4160d7851d032ba8`, the signature
`0x63d0c910…c52901bd00`; the registry data is 280 bytes without and 304 bytes with endpoint
`https://203.0.113.7:8131`. The test builds both data forms with the test builder, checks them byte for byte,
runs the contract's own parser and attestation check (`csr-registry/src/attest.rs`) on them (the signature
recovers to the vector's monitor key), and registers an entry signed by the vector's monitor key in the VM.

## 3. Attestation message `CSR-ATTEST-v2` (the monitor implements the same)

```
message   = blake2b-256( "CSR-ATTEST-v2" ‖ molecule(AttestationBodyV2) )
signature = r (32) ‖ s (32) ‖ recovery_id (1)         // recoverable secp256k1 over `message`
```
* blake2b-256 uses the CKB personalization `ckb-default-hash`.
* `"CSR-ATTEST-v2"` is the 13 ASCII bytes without terminator.
* `molecule(AttestationBodyV2)` is the standard molecule table encoding (header with total size and six field
  offsets, then the fields in order). The fields come from the **registry cell itself**: its `peer_id`,
  `service_pubkey`, `network`, `expiry_ms` and `stake`, and `registry_type_hash` = the hash of the registry
  cell's own type script (`load_script_hash` in `csr-registry`; the job lock's args in `csr-job-lock`).
* Binding the stake and the registry type hash (D21 A, F-CON-002, F-CON-005): an attestation cannot be used
  with another deposit or with another registry deployment (other code or other `config_id`). A v1 signature
  (`CSR-ATTEST-v1` over peer id, key, network and expiry) does not verify (error 10).
* `recovery_id` is 0–3; `s` must be low (≤ n/2, as produced by libsecp256k1 / `ckb-crypto`); `r`, `s` big endian.
* The recovered compressed public key must equal `monitor_pubkey` of the config cell.
* Verified in-script with the pure-Rust `k256` 0.13 crate (`VerifyingKey::recover_from_prehash`); the message is
  hashed with `blake2b-ref`. Shared code: `contracts/csr-registry/src/attest.rs` (registry, job lock, tests).

Byte layout of the hashed input for `peer_id` of length *p*:

| offset | size | content |
|-------:|-----:|---------|
| 0 | 13 | `CSR-ATTEST-v2` |
| 13 | 4 | total size of the body = 28 + 4 + p + 33 + 1 + 8 + 36 + 32 = 142 + p (u32 LE) |
| 17 | 4 | offset of `peer_id` = 28 |
| 21 | 4 | offset of `service_pubkey` = 32 + p |
| 25 | 4 | offset of `network` = 65 + p |
| 29 | 4 | offset of `expiry_ms` = 66 + p |
| 33 | 4 | offset of `stake` = 74 + p |
| 37 | 4 | offset of `registry_type_hash` = 110 + p |
| 41 | 4 + p | `peer_id` (u32 LE length ‖ bytes) |
| 45 + p | 33 | `service_pubkey` |
| 78 + p | 1 | `network` |
| 79 + p | 8 | `expiry_ms` (u64 LE) |
| 87 + p | 36 | `stake` (`tx_hash` ‖ `index` u32 LE) |
| 123 + p | 32 | `registry_type_hash` |

(For the vector, *p* = 48: body 190 bytes.)

## 4. Rules

On **creation and on every update** (for each registry output):

1. Exactly one cell dep has the config cell's type hash (`csr-config-type`, `data2`, `args = config_id`, §1)
   (error 8); its data is at least 41 bytes (error 9).
2. `attestation_sig` recovers to that cell's `monitor_pubkey` over the `CSR-ATTEST-v2` message of the output's
   fields and the registry's own type hash (§3) (error 10).
3. *(v2: "the attestation copy equals the registry data", error 11 – gone since v3: the signature is over the
   cell's own fields, so a mismatch is a signature failure, error 10.)*
4. `expiry_ms > clock_ms` (error 12) and `expiry_ms − clock_ms ≤ 8 × 86,400,000` ms = **8 days** (error 24,
   "attestation lifetime over the cap"; D24 A). The monitor issues attestations for at most 7 days; the cap
   leaves a day for clock lag.
5. The output lock is `csr-dual-lock` whose runner hash equals the lock hash of
   `secp256k1_blake160_sighash_all` with `args = blake160(service_pubkey)`.
   * "Is `csr-dual-lock`": `code_hash` = data hash of the `csr-dual-lock` binary of the same build (embedded into
     `csr-registry` at build time by `csr-registry/build.rs`), `hash_type` `data1` or `data2`, args **64 bytes**
     (`owner ‖ runner`) or **32 bytes** (runner-owned: owner = runner = the args). The runner hash is the last
     32 bytes of the args, the owner hash the first 32.
   * Accepted forms of the sighash lock: `code_hash 0x9bd7e06f…a3cce8` with `hash_type type` (the genesis
     deployment on mainnet, testnet and dev chains), or the binary's data hash `0x709f3fda…e08649`
     (ckb-system-scripts 0.5.4) with `hash_type data`, `data1` or `data2`. All four run the same code.

6. **Runner key present** (creation, and owner-path updates that change `service_pubkey` or the output's runner
   lock hash): the transaction has an input whose lock hash equals the **output's** runner hash, i.e. the
   node's runner key cell. This proves possession of the service key (error 17). It is checked after rules 1–5.

### 4.1 Stake rules

The stake of a registration is a **referenced Nervos DAO deposit** owned by the operator (decision D-A7-1).
Its owner, the **stake owner** (the lock hash of the deposit cell), need not be the **registry owner** (the
dual lock's `owner_lock_hash`); since A8 the stake owner consents by a signed message instead (decision D-A8-1,
§4.2).
CSR never holds or moves it; the registry only points to it with `stake`. Constants (hard-coded in
`csr-registry`):

* `MIN_STAKE` = **10,000 CKB** = 1,000,000,000,000 shannons.
* Nervos DAO type script: `code_hash 0x82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e`,
  `hash_type type`, empty args (the same on every chain).

The stake rules are checked on **creation** and on an **owner-path update that changes `stake`, the owner**
(the output's owner hash differs from the input's), **`peer_id`, `network` or `service_pubkey`** (decision
D23 A, F-CON-004: the consent text names these fields, so a new node identity needs a new consent of the stake
owner; an input whose data is not valid v3 counts as changed). They run after rules 1–6. On the runner path
these fields are fixed (error 14), so a runner refresh needs no stake cell dep; neither does an owner update
that only changes `expiry_ms`, `attestation_sig`, `payout_lock_hash`, `endpoint` or `prekeys`.

**Stake window (decision D22 C).** The stake is checked on creation and on those changes **only**, not on a
refresh or a settlement. A deposit that is withdrawn (spent into a withdrawing cell) after the registration
therefore **keeps earning until its attestation expires**: the monitor stops attesting the registration, the
runner cannot refresh it, and the last attestation runs out – **at most 7 days** later as issued by the
monitor, and at most **8 days** by the on-chain cap (rule 4). Consumers that need a live deposit check it
themselves (the out point is in the data).

1. The transaction's `cell_deps` – read from the raw transaction (`load_transaction`; `RawTransaction.cell_deps`
   is a fixvec of 37-byte `CellDep` = `OutPoint (36) ‖ dep_type (1)`) – contain an entry equal to `stake` with
   `dep_type` **code** (0), and **every entry before it is also code**. Its position *j* in the raw list is then
   its index in `Source::CellDep` (a `dep_group` entry expands to its members, which would shift the indices).
   Builders put the stake **first**. Otherwise **error 18**.
2. The cell at `Source::CellDep` index *j* has the Nervos DAO type script above and data of **exactly 8 zero
   bytes** (a deposit; a phase-1 withdrawing cell holds a block number instead). Otherwise **error 19**.
3. Its capacity ≥ `MIN_STAKE`. Otherwise **error 20**.
4. **The stake owner consents** (A8; replaces A7's "its lock hash equals the registry owner"). The stake owner
   is the lock hash of the cell at `Source::CellDep` index *j*; the registry owner may be any lock. Checked in
   this order:
   1. If an input has the stake owner's lock hash, the consent is given (the A7 way: a wallet-signed
      transaction; kept for other tools).
   2. Otherwise the witness of the registry output (`load_witness_args(0, Source::GroupOutput)`, i.e. the
      witness at the registry output's index) must be a `WitnessArgs` with an `output_type`. No witness, a
      witness that is not `WitnessArgs`, or no `output_type` → **error 21** ("the stake owner did not consent").
      A signature in `lock` or `input_type` does not count.
   3. The `output_type` is the consent signature (§4.2). Not exactly 65 bytes, a recovery byte outside 0–3 and
      27–30, or `r`/`s` not a valid scalar → **error 22**.
   4. The public key recovered from the signature over the consent digest (§4.2) gives
      `args = blake160(compressed key)`; the lock hash of one of the four accepted `secp256k1_blake160_sighash_all`
      forms (rule 5) with these args must equal the stake owner. Anything else – another text, key, stake,
      service key, peer id or network, a digest without the `Nervos Message:` prefix, a signature that recovers
      no key – → **error 23**.

A cell dep must be live, so the deposit exists at the moment the registration is written. When the owner is
the runner (§9), every update the runner signs takes the owner path; a refresh that keeps `stake` and the owner
still needs neither the stake dep nor a consent. Why the out point
must be matched in the raw transaction: a script can load a dep cell's fields but not its out point, so the
only way to prove that the dep *is* the registry's `stake` is to find `stake` in the raw `cell_deps` and map its
position to the `Source::CellDep` index – which is only possible while no dep group precedes it.

**Checked later by the CSR Network Monitor** (a script cannot see later spends or other registrations):

* the deposit is **still deposited** (the out point is live, not spent into a withdrawing cell); a registration
  whose stake has been spent is not attested any more and drops out once its attestation expires;
* **one deposit per registration**: when several registry cells name the same `stake`, only the **oldest**
  registration counts (oldest wins); the monitor does not attest the others.

### 4.2 Stake consent (signed message)

The stake owner signs a **one-line text** in its wallet ("Sign message" in Neuron). The text and the digest are
built by `contracts/csr-registry/src/consent.rs` (`no_std`), which the contract tests include via `#[path]`.

**Text** (UTF-8, fields separated by `|`, no trailing newline):

```
CSR-STAKE-CONSENT-v1|<network>|0x<stake tx hash>:<stake index>|0x<service_pubkey>|<peer_id>
```

* `<network>`: `mainnet`, `testnet` or `devnet` for registry `network` 0, 1, 2.
* `<stake tx hash>`: 64 lower-case hex digits; `<stake index>`: decimal (no leading zeros); both from `stake`.
* `<service_pubkey>`: 66 lower-case hex digits (33 bytes, compressed).
* `<peer_id>`: the raw bytes of `peer_id` (the libp2p peer id string).

All values come from the **registry output's** data, so a consent is bound to one network, one deposit, one
service key and one node.

Example (network testnet, stake `0x5a…5a` index 3, service key
`0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27`, peer id
`QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB`):

```
CSR-STAKE-CONSENT-v1|testnet|0x5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a:3|0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27|QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB
```

**Digest** (Neuron's "Sign message" format): `blake2b-256("Nervos Message:" ‖ text)` with the CKB
personalization `ckb-default-hash` and no space after the colon. For the example:
`0xe3425937793f1e3c6de997e908df0c8ac00efdfd8db48948adca4d4e9a4b51c2` (`registry_consent_text_vector`).
Cross-check against a real Neuron signature (`registry_consent_neuron_vector`): the message `Message from CSR`
has digest `0x2bb15d28fd5e98471846af625ba4e735e60491793d2159cd595331cb81a1a8a0`; Neuron's signature
`0xc0fee686…2877f401` recovers to a key whose `blake160` is `0x89c04afd2ded4725e915020cb757a20d353f1094`.

**Signature:** 65 bytes `r ‖ s ‖ v`. `v` 0–3 is accepted, and 27–30 (minus 27). A high-S signature is
normalised first (`s → n − s`, the low bit of the recovery id flipped), so both forms are accepted.

**Where it sits:** `WitnessArgs.output_type` of the witness at the registry output's index. CSR always puts the
registry output at index 0 and its runner wallet cell at input 0, so witness 0 carries both the runner's
signature (`lock`) and the consent (`output_type`); `output_type` is set before signing, so the runner's
sighash-all signature covers it.

**Stake owner vs. registry owner:** the stake owner (the deposit's lock) consents to *this* registration using
its deposit; the registry owner (dual-lock `owner_lock_hash`) controls the registry cell (owner path). They may
be the same lock (A7) or different (A8: the runner owns the cell, the operator's wallet owns the deposit).

CSR defines **no slashing and no job collateral**. The stake is a sybil cost and a signal for consumers;
applications that need a bond define it in their own contracts.

Data must be a complete molecule `RegistryV3` with exactly the schema's field count (10 fields; no extra
fields) and field sizes (`service_pubkey` 33, `payout_lock_hash` 32, `stake` 36, `expiry_ms` 8,
`attestation_sig` 65 bytes), `version = 3`, `network` ≤ 2 and a valid `endpoint` (§2); otherwise error 7.

**One registry input and one output per group** (1:1; creation: 0 → 1, destruction: 1 → 0).

**Owner path** (an input with the input's owner hash present): any field may change; rules 1–5 still apply
to the output. If `service_pubkey` or the runner lock hash changes, rule 6 requires the runner key cell of the
**new** runner lock hash (the old service key does not authorise a new one). If `stake`, the owner, `peer_id`,
`network` or `service_pubkey` changes, the stake rules (§4.1) apply to the output: the stake owner (of the new
or the kept deposit) must consent again, by input or by signature (**consent rule**, D23 A).

**Runner path** (no owner input): only `expiry_ms`, `attestation_sig`, `endpoint` and `prekeys` may change.
`version`, `peer_id`, `service_pubkey`, `payout_lock_hash`, `network`, `stake`, lock and type stay identical
(error 14); output capacity ≥ input capacity (error 15); the cell must not be destroyed (error 16).

**Destroying** (input without registry output) requires the owner path.

For *creation* there is no input to read an owner from; the output's lock args carry the owner hash. Rules 1–6
and the stake rules apply: anybody may register **their own** key (sybil resistance is the monitor's attestation), but only the
holder of the service key can create an entry for it (rule 6).

Why rule 6 (audit finding **F1**): attestations are public on chain. Without it anyone could copy another
node's attestation and create a second registry cell with that node's `peer_id` and `service_pubkey` but their
own owner, `payout_lock_hash` and `prekeys`, then collect tips (or publish fake prekeys) under that identity.

Note (**F8**): the same key holder can still create several registry cells for one service key. Off-chain
consumers (CSR, monitor, CRT) must pick the one with the **latest** attestation (`expiry_ms`) and ignore the
others.

## 5. Error codes

| code | meaning |
|-----:|---------|
| 1–4 | ckb-std syscall errors |
| 5 | type args are not 32 bytes |
| 6 | more than one registry input or output in the group |
| 7 | registry data is not valid `RegistryV3` (v1/v2 table, `version` ≠ 3, `network` > 2, bad `endpoint`) |
| 8 | config cell dep missing or not unique (type hash of `csr-config-type`, `data2`, `args = config_id`) |
| 9 | config cell data malformed (< 41 bytes) |
| 10 | attestation signature invalid or not by the monitor over `CSR-ATTEST-v2` of this cell and this registry type (a v1 signature, another stake, network, peer id, key or registry type) |
| 11 | unused since v3 (was: attestation fields differ from registry data) |
| 12 | attestation expired (`expiry_ms <= clock_ms`) |
| 13 | output lock is not `csr-dual-lock` (args 32 or 64 bytes) or its runner hash does not match `service_pubkey` |
| 14 | runner path changed a forbidden field |
| 15 | runner path reduced capacity |
| 16 | destroy without owner path |
| 17 | no input with the output's runner lock hash (creation, or owner path changing the service key) |
| 18 | stake cell dep missing: no `code` cell dep equal to `stake`, or a non-code (dep group) entry before it, or `stake` itself referenced as dep group |
| 19 | stake cell is not a Nervos DAO deposit (type script not the DAO with `hash_type type` and empty args, or data not 8 zero bytes) |
| 20 | stake capacity below `MIN_STAKE` (10,000 CKB) |
| 21 | the stake owner did not consent: no input with the stake's lock hash and no `output_type` in the registry output's witness (missing, not `WitnessArgs`, or no `output_type`) |
| 22 | consent signature malformed: `output_type` not 65 bytes, recovery byte not 0–3/27–30, or `r`/`s` invalid |
| 23 | consent signature not by the stake owner over this registration's text (another key, stake, service key, peer id or network, no `Nervos Message:` prefix, or no key recoverable) |
| 24 | attestation lifetime over the cap: `expiry_ms − clock_ms` > 8 days (D24 A) |

## 6. Worked examples

*Create.* The operator has made a Nervos DAO deposit of at least 10,000 CKB with the owner wallet beforehand
(the wallet does that; CSR does not). Inputs `[owner wallet cell, runner key cell]`, outputs `[registry cell
(dual-lock, type registry, data with fresh attestation, `payout_lock_hash` of the payout cell, `stake` = the
deposit's out point), change, runner key cell]`, cell deps `[stake deposit, registry code, config cell, …]`
(the stake first, all `code`; the secp256k1 dep group, if any, after it). Both inputs are signed (owner wallet
by its lock, runner key cell with the service key). The runner key cell may also be the only input if it pays
for the registry cell. The deposit is only referenced, not spent: it keeps earning DAO interest.

*Rejected creation:* owner wallet only (17, `registry_create_without_runner_key_rejected`); attacker as owner
with the victim's `peer_id`, `service_pubkey` and copied attestation, the attacker's payout cell and stake and
the attacker's own key cell (10 since v3: the signature covers the stake; `registry_create_with_copied_attestation_rejected`);
an attestation for another stake or registry deployment (10, `registry_attest_v2_other_*_rejected`), a v1
signature (10, `registry_attest_v1_signature_rejected`), `expiry_ms` more than 8 days ahead of the clock (24,
`registry_attestation_lifetime_over_cap_rejected`); no stake dep (18,
`registry_create_without_stake_dep_rejected`); a dep group before the stake (18,
`registry_create_stake_dep_after_dep_group_rejected`); a withdrawing DAO cell (19,
`registry_create_stake_withdrawing_rejected`); 9,999.99999999 CKB (20, `registry_create_stake_below_min_rejected`);
another key's deposit without its consent (21, `registry_create_stake_of_other_owner_without_consent_rejected`);
a consent signature of another length (22) or for another stake, service key, peer id or network (23,
`registry_create_consent_for_other_*_rejected`).

*Runner refresh.* Inputs `[registry cell, runner key cell]`, outputs `[registry cell with new `expiry_ms` and
`attestation_sig` (and optionally a new `endpoint` or `prekeys`; other fields the same, capacity ≥), runner key
cell]`, cell deps `[config cell]` (no stake dep needed; `registry_runner_refresh_v3_ok`,
`registry_runner_changes_endpoint_ok`).

*Owner changes the node identity* (`peer_id`, `network` or `service_pubkey`). Like a stake change: cell deps
`[stake deposit, …]` and the stake owner's consent for the new text (`registry_owner_changes_peer_id_with_consent_ok`;
without the consent → 21, `registry_owner_changes_{peer_id,network,service_pubkey}_without_consent_rejected`).

*Owner changes the payout cell.* Inputs `[registry cell, owner wallet cell]`, outputs `[registry cell with new
`payout_lock_hash`]` (no stake dep needed).

*Owner moves the stake.* Inputs `[registry cell, owner wallet cell]`, outputs `[registry cell with the new
deposit as `stake`, change]`, cell deps `[new stake deposit, registry code, config cell, …]`
(`registry_owner_changes_stake_with_new_deposit_ok`; without the dep → 18). To withdraw the old deposit the
operator moves the stake (or destroys the registration) first; otherwise the monitor stops attesting it.

*Owner transfers the registration.* As above with dual lock `new owner ‖ runner hash` and a deposit of the
**new** owner as `stake` and the new owner's consent signature (`registry_owner_changes_owner_with_new_deposit_ok`).
An owner change keeps the stake only with the stake owner's consent: its cell as an input
(`registry_owner_changes_owner_with_stake_owner_input_ok`) or a signature; without → 21
(`registry_owner_changes_owner_without_stake_consent_rejected`).

*Owner rotates the service key.* Inputs `[registry cell, owner wallet cell, new runner key cell]`, outputs
`[registry cell with new `service_pubkey`, a fresh attestation for it and dual lock `owner ‖ new runner hash`,
change, new runner key cell]`, cell deps `[stake deposit, …]` and the stake owner's consent (here the owner's
own deposit, consent by input) (`registry_owner_changes_service_pubkey_with_new_runner_key_ok`). Without the
new key cell, or with only the old one, → 17.

*Owner destroy.* Inputs `[registry cell, owner wallet cell]`, outputs `[change]`.

Owner presence on update/destroy is taken from the **input** cell's lock args (owner hash = the first 32
bytes of its dual-lock args; with 32-byte args the args themselves). On the owner path the output may carry a different dual lock (e.g. a new owner),
as long as rule 5 holds.

## 7. Use by the job lock

`csr-job-lock` (args = `registry_type_hash`, the hash of this registry type script) reads a registry cell as a
**cell dep** (`RegistryV3`; anything else is error 10) and takes its `payout_lock_hash` as the lock of the cell
that must receive the tip (`job-cell-standard.md`): the net inflow (Σ outputs − Σ inputs) to cells with that
lock hash must be ≥ the tip, else error 11. It requires (F2) an input with the registry cell's runner hash
(dual-lock args of 64 or 32 bytes, the node's runner key cell), and (F5, D21 A, D24 A) exactly one config cell
dep (the `csr-config-type` type hash for the registry's `config_id`) by which the attestation is valid: the
signature by `monitor_pubkey` over `CSR-ATTEST-v2` with `registry_type_hash` = its own args (error 10),
`expiry_ms > clock_ms` (error 17) and `expiry_ms − clock_ms ≤ 8 days` (error 18). A registry cell whose
attestation has expired therefore earns nothing until its runner refreshes it via the runner path. Since
`payout_lock_hash` can only be changed by the owner path, a compromised runner key cannot redirect tips of an
operator-owned registration. Since A8 `payout_lock_hash` is the runner wallet itself (§9) and the settlement's
runner wallet input receives the tip.

## 8. Cycles

Measured with ckb-testtool (`registry_create_cycles_of_type_script`, `registry_v3_create_ok`,
`registry_create_with_consent_signature_ok`, `registry_create_owned_by_runner_ok`, `registry_runner_refresh_v3_ok`,
`job_tip_to_runner_wallet_ok`, `config_clock_forward_ok`; peer ID of 40 bytes, release build `opt-level = "s"`;
contracts v3):

| transaction | cycles |
|-------------|-------:|
| registry creation, `csr-registry` type script alone, consent by signature (tx minus a key-cell-only baseline) | 15,835,892 |
| registry creation, whole tx, consent by input (type script + two system-lock inputs: wallet, runner key) | 11,364,077 |
| registry creation, whole tx, consent by signature (type script + two system-lock inputs) | 19,316,645 |
| runner-owned registration, whole tx (type script + one runner wallet input, consent by signature) | 17,607,081 |
| runner refresh, whole tx (type script + dual lock + system lock) | 9,646,669 |
| settlement into the runner wallet, whole tx (job lock + job type + system lock) | 9,622,456 |
| config clock update, whole tx (`csr-config-type` + system lock) | 1,734,476 |

Almost all of it is secp256k1 public-key recovery in `k256` (one for the attestation, one more for a consent
signature; ~8M each), whose cost varies with the signature. Since v3 the job lock verifies the attestation
signature too (D21 A), so a settlement costs one recovery more than in A8 (1,744,766). All stay far below the
70M limit used by the tests. Binaries: `csr-registry` 96,088 bytes, `csr-job-lock` 93,680, `csr-config-type`
23,488, `csr-job-type` 9,320, `csr-dual-lock` 1,944.

## 9. Runner wallet (A8)

The **runner wallet** is the set of plain cells (no type, empty data) of the runner's own
`secp256k1_blake160_sighash_all` lock (args = `blake160(service_pubkey)`; A7's "runner key cell"). Decision
D-A8-1: CSR signs every transaction itself with the service key; the operator only funds the wallet once and
signs the consent text (§4.2) in its wallet.

* **The runner owns its registration:** dual-lock args = `runner_lock_hash` (32 bytes, since v3; 125 CKB less
  together with the smaller data) or `runner_lock_hash ‖ runner_lock_hash`. Every update the runner signs is
  an owner-path update; rules 1–6 and (on a stake, owner or identity change) the stake rules still apply
  (`registry_runner_owned_short_args_ok`).
* **Tips go into the runner wallet:** `payout_lock_hash = runner_lock_hash`. The job lock is unchanged: the
  settlement already spends a runner wallet cell (F2), and its output now holds that cell's capacity plus the
  tip; the fee comes from the job cell as before, so the wallet grows by exactly the tips.
* The registry output is always output 0 and the first runner wallet cell is always input 0, so witness 0 holds
  the runner's signature (`lock`) and, when needed, the consent (`output_type`). The wallet change cell keeps
  ≥ 61 CKB.

Worked examples (tests in `contracts/csr-contract-tests/tests/{registry,job}.rs`):

*Register (create)* (`registry_create_owned_by_runner_ok`). Inputs `[runner wallet cell(s)…]`, outputs
`[registry cell (dual lock `runner` (32 bytes) or `runner ‖ runner`, data: `payout_lock_hash = runner_lock_hash`,
`stake`, attestation, peer id, network, endpoint), wallet change]`, cell deps `[stake deposit, registry code, config cell, secp256k1 dep group]`
(the stake first), witness 0 = `WitnessArgs { lock: runner signature, output_type: consent signature }`.

*Register (update, new stake)* (`registry_runner_owned_changes_stake_with_consent_ok`; without the consent → 21).
Inputs `[runner wallet cell(s)…, registry cell]`, outputs `[registry cell with the new stake and the given
attestation, wallet change]`, cell deps `[new stake deposit, …]`, consent in witness 0. The same registry cell is
updated, e.g. after the monitor disqualified a withdrawn deposit.

*Refresh* (`registry_runner_owned_refresh_ok`). As the update, keeping `stake`: no stake dep, no consent.
Inputs `[runner wallet cell(s)…, registry cell]`, outputs `[the same registry cell with only the attestation
replaced, wallet change]`, cell deps `[registry code, config cell, dual lock, secp256k1 dep group]`. CSR builds
it with `csr register refresh` (or "Renew now" / `POST /api/registry/refresh`): it asks the monitor for a new
attestation for the same node, key and stake and refuses one for another peer ID or key, or one that does not
expire later than the current one.

*Automatic renewal.* While the runtime is not `off`, `csr run` reads the registry cell and the config cell clock
every 30 minutes. When the cell is runner-owned and the attestation expires within 2 days (`RENEW_BEFORE_MS`,
measured on the chain clock of the config cell, not the machine clock), it renews it as above. A failure is
logged as a warning, shown on the dashboard (`renewal_error`) and retried after 30 minutes. The refresh spends a
runner wallet cell, so, like a sweep, it is followed until committed (settings key `refresh_pending`); new job
batches wait while it is pending, and it is given up after `SWEEP_PENDING_AFTER_BLOCKS` blocks. A registration
owned by the operator (A7) cannot be renewed by the runner; only the owner's wallet can.

*Remove* (`registry_runner_owned_destroy_ok`). Inputs `[runner wallet cell, registry cell]`, outputs `[wallet
cell with everything minus the fee]`: the registry cell's capacity returns to the runner wallet.

*Settlement* (`job_tip_to_runner_wallet_ok`). Inputs `[job cell, runner wallet cell]`, outputs `[job cell
recreated or refund cell (at the job input's index, F-CON-003), runner wallet cell with the same capacity + tip]`, cell deps `[registry cell, config cell,
…]`. If the wallet cell also pays the fee, the net inflow to `payout_lock_hash` is below the tip → job lock
error 11 (`job_tip_to_runner_wallet_paying_fee_rejected`).
