# Publishing a service

For app developers. After this guide your service shows up in every CSR runner's catalog; the operators then add,
approve and enable it. **No runner update is needed** (D52): you deploy the code (and data) cells, publish a manifest
and post job cells. Spec: `spec/service-manifest.md`, `spec/host-interface.md`, `spec/job-cell-standard.md`.

You need a CKB key (secp256k1) with enough CKB: the code cell, the manifest cell (about 450 CKB) and the fee. The
manifest can only be published by the lock that **owns the code cell**, so deploy the code with that same key.

## 1. Build

* Rust or C: build a RISC-V program for CKB-VM (`bash scripts/build-services.sh` builds the examples in `services/`).
  Use the host calls of `spec/host-interface.md`. Test it locally without a chain:
  `csr service run-local --manifest services/manifests/hello-rust.toml --payload 0x0102`.
* JavaScript: run it on `ckb-js-vm` (`services/vendor/ckb-js-vm`). The program is the VM, the script is a **data
  cell**, and the args tell the VM which data cell to load (`services/manifests/hello-js.toml`).

## 2. Deploy the code cell (and data cells)

Put every binary and data file in its own cell of your lock (for example with `ckb-cli`, Neuron or
`csr-dev deploy-service` on devnet/testnet). Note the out points `0x<tx hash>:<index>`. Wait until they are
committed.

## 3. Publish the manifest

```
csr service publish --code-outpoint 0x<tx>:0 \
    [--data-outpoint 0x<tx>:0]… [--args 0x<hex>] \
    --name hello-rust --version 1.0.0 \
    --description "Says hello" [--description-file F] \
    [--url https://example.org] [--source https://github.com/you/repo] [--language rust] [--min-csr 0.6.0] \
    --key key.txt [--network testnet|devnet] [--dry-run]
```

`key.txt` holds the private key as hex (the publisher's key, not the runner wallet; `csr` never stores it). The
command builds the transaction (Type ID from the first input, the code and data cells as cell deps, change back to
you, fee 2 shannons per byte + 1,000), checks it with your node (`test_tx_pool_accept`), sends it (not with
`--dry-run`) and prints JSON with the **Type ID**, the **service ID** and the **manifest out point**. Configure the
node with `--config` (`[node] rpc_url`) as for every `csr` command. Mainnet is refused until the manifest script is
deployed there.

Limits: name 1–48 bytes, version 1–16, description up to 512 bytes of plain text (no HTML, no Markdown), `url` and
`source` empty or `https://` up to 128 bytes, language up to 16, args up to 64 bytes, up to 4 data cells.

Later:

```
csr service publish --update TYPE_ID --version 1.1.0 --description "…" --key key.txt   # only the given fields change
csr service publish --update TYPE_ID --deprecate --key key.txt                          # mark as deprecated
csr service publish --destroy TYPE_ID --key key.txt                                    # remove it, the CKB returns to you
```

A new binary, other args or other data cells are a **new manifest** (new Type ID, new service ID).

## 4. The service ID and job cells

* No args and no data cells: the **service ID is the code hash** (the data hash of the program), as before.
* Otherwise: `blake2b_256("CSR-SERVICE-v1" ‖ code_hash ‖ u32 LE len(args) ‖ args ‖ u8 n ‖ data_hash_1 … data_hash_n)`.

`csr service publish` prints it. Create job cells with `args = service ID` in `csr-job-type`
(`spec/job-cell-standard.md`; on devnet `csr-dev job --code-hash <service ID> …`). The contract does not interpret
the 32 bytes.

## 5. What operators see

Within a day (at start, daily and on "Refresh") every runner lists your manifest on the Services page with name,
version, description and links, and the Overview shows **"New service available: <name>"** until the operator opens
the Services page or dismisses it. The operator clicks *Add* (or *Add & approve*); only then does the runner run your
jobs. Links are shown as text and, for `https://` URLs only, as links; runners never fetch them. If you update the
manifest, runners that already added the service show "Update available" on its row. If jobs name a service ID that
has no manifest, runners list it as "seen on chain" without a description.

A runner update is only needed for changes to the runner itself: a new host call (7001–7099), a new trigger kind,
new contract versions or higher fixed limits. Set `--min-csr` if your service needs a newer runner.
