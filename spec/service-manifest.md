# CSR service manifest (`csr-service-manifest`)

Status: new (csr-service-catalog, decisions D47–D52). Independent of the other contracts: `csr-registry`,
`csr-job-type`, `csr-job-lock`, `csr-dual-lock` and `csr-config-type` do not change.

## 1. Purpose

Nothing on chain says that a code cell is a CSR service, what it does or who published it. A **manifest cell** does:
one cell per service with name, version, description, links, the code cell and, for services that need them, data
cells and script args. Runners read manifests from their own node (indexer `get_cells` on the manifest type script,
D47) and list them in their catalog. A manifest is information only: **nothing runs without the runner admin's
click.**

The type script binds a manifest to its code: only the lock that owns the code cell can publish its manifest
(publisher binding).

## 2. Cell

* Type script `csr-service-manifest`, `hash_type = data2`, `args = type_id (32)` (D48).
* Lock: the publisher's ordinary lock (it authorises update and destruction).
* Data: molecule `ServiceManifestV1` (`contracts/schemas/csr.mol`):

```
table ServiceManifestV1 {
    version_tag: byte,          // 1
    code_hash:   Byte32,        // data hash of the program (data2)
    code_cell:   OutPoint,      // where the program lives
    data_hashes: Byte32Vec,     // 0..4 data cells, in cell-dep order
    data_cells:  OutPointVec,   // same length and order as data_hashes
    args:        Bytes,         // 0..64 bytes, passed as the service script args
    name:        Bytes,         // UTF-8, 1..48 bytes
    version:     Bytes,         // UTF-8, 1..16 bytes, e.g. "1.0.0"
    description: Bytes,         // UTF-8, 0..512 bytes, plain text
    url:         Bytes,         // empty or https:// URL, <= 128 bytes
    source:      Bytes,         // empty or https:// URL (e.g. GitHub), <= 128 bytes
    language:    Bytes,         // UTF-8, <= 16 bytes ("rust", "c", "js", ...)
    min_csr:     Bytes,         // empty or "X.Y.Z", the first runner version that can run it
    deprecated:  byte,          // 0 or 1
}
```

Text only; descriptions are plain text (no HTML, no Markdown). Runners never fetch `url` or `source` (D50).

## 3. Rules

Every output of the group (at most one) must hold valid data; then, by case:

1. **Args** are exactly 32 bytes (error 5).
2. **Type ID** (`ckb_std::type_id::check_type_id(0, 32)`, the same rule as `csr-config-type`): at most one input and
   one output carry this type script, and a new cell's `args` are `blake2b-256(first input ‖ output index u64 LE)`
   (error 6).
3. **Creation** (no input in the group): a **cell dep** whose out point equals `code_cell` and whose
   `blake2b(data) == code_hash` exists (error 15); that cell's **lock hash equals the manifest output's lock hash**
   (publisher binding, error 16); every `data_cells[i]` is a cell dep with `blake2b(data) == data_hashes[i]`
   (error 17). These deps must be plain code deps (dep type `code`) and no dep group may precede them in the
   transaction's cell dep list.
4. **Update** (one input, one output, same Type ID): `code_hash`, `code_cell`, `data_hashes`, `data_cells` and `args`
   may not change (a new binary is a new manifest, error 18); the lock may not change (error 19); `name`,
   `version`, `description`, `url`, `source`, `language`, `min_csr` and `deprecated` may change.
5. **Destruction** (input, no output): allowed; the lock authorises it.

### 3.1 Error codes

| code | meaning |
|-----:|---------|
| 1–4 | syscall failure (index out of bound, item missing, length not enough, other) |
| 5 | script args are not 32 bytes |
| 6 | Type ID rule violated |
| 7 | more than one input or output of the group |
| 8 | data is not a well-formed `ServiceManifestV1` |
| 9 | `version_tag` is not 1 |
| 10 | a length cap is violated (name 1..48, version 1..16, description ≤ 512, args ≤ 64, language ≤ 16, url/source ≤ 128, min_csr ≤ 16) |
| 11 | a text field is not valid UTF-8 |
| 12 | `url` or `source` is not empty and does not start with `https://` |
| 13 | `data_hashes.len() != data_cells.len()` or more than 4 data cells |
| 14 | `deprecated` is not 0 or 1 |
| 15 | creation: no cell dep with the out point `code_cell` and data hash `code_hash` |
| 16 | creation: the code cell's lock is not the manifest's lock (publisher binding) |
| 17 | creation: a data cell is not a cell dep with the stated out point and data hash |
| 18 | update: `code_hash`, `code_cell`, `data_hashes`, `data_cells` or `args` changed |
| 19 | update: the lock changed |
| 20 | `min_csr` is not empty and not `X.Y.Z` (digits) |

## 4. Service ID (D49)

The service ID is what job cells name in `csr-job-type` args (32 bytes, unchanged; the contract does not interpret
it):

* `args` empty and no data cells: **service ID = `code_hash`**, exactly as before (single binary).
* otherwise: `blake2b_256("CSR-SERVICE-v1" ‖ code_hash ‖ u32 LE len(args) ‖ args ‖ u8 n ‖ data_hash_1 … data_hash_n)`
  with the CKB default personalization `ckb-default-hash`.

The keeper loads the code cell, the data cells and the args from the chain.

## 5. Capacity

A manifest cell occupies about `8 (capacity) + 53 (lock) + 65 (type) + data` bytes. With a lock of 53 bytes and a
300-byte description the data is about 380 bytes, so about 450 CKB. Destroying the cell returns the capacity.

## 6. How to publish

1. Deploy the code cell (and the data cells) with your lock. The manifest must be created with the same lock.
2. Build the `ServiceManifestV1` data and the Type ID (`blake2b-256(first input ‖ output index)`).
3. Create a transaction with the code cell (and every data cell) as **cell deps**, one output with the manifest's
   type script and your lock. Sign with your key.
4. Post job cells whose `csr-job-type` args are the service ID.
5. To release a new version of the text, update the manifest (same input and output lock). A new binary is a new
   manifest with a new Type ID.

## 7. Guarantee and limits (D52)

After this release, publishing a new service needs **no runner update**: the developer deploys the code (and data)
cells, publishes a manifest and posts job cells; every runner sees it in its catalog within a day. The admin still
adds, approves and enables it.

A runner update stays necessary only for changes to the runner itself: a new host call (7001–7099), a new trigger
kind, new contract versions, or higher fixed limits. `min_csr` tells admins which runner version a service needs.
