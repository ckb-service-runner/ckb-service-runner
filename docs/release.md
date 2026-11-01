# Making a release

Operators update from the dashboard ("Check for update" → "Update") or with `csr update stage`.
The runner downloads the binary for its machine, `SHA256SUMS` and `SHA256SUMS.sig` from the latest
GitHub release of `ckb-service-runner/ckb-service-runner`, checks the ed25519 signature of
`SHA256SUMS` against the release key pinned in the binary (`RELEASE_PUBKEY` in
`runner/src/update.rs`) and the SHA-256 of the binary, and stages it. The root installer
(`csr-update.path`, see `docs/install.md`) lets the installed binary check it again and installs it.

## The release key

- One ed25519 key pair. The private key lives **offline** (an air-gapped machine or a hardware
  token, plus an encrypted backup); it never enters this repository, CI or AIWorker.
- Create it once:

  ```sh
  openssl genpkey -algorithm ed25519 -out csr-release.key      # on the offline machine
  openssl pkey -in csr-release.key -pubout -outform DER | tail -c 32 | xxd -i
  ```

  The last command prints the 32 bytes of the public key. Pin them in `RELEASE_PUBKEY`
  (`runner/src/update.rs`). While `RELEASE_PUBKEY` is all zeros every update is refused
  ("no release key configured").
- Changing the key: release a version with the new public key, signed with the old key. A build
  only accepts releases signed with the key pinned in it.
- If the private key is lost, operators must install the next version manually; if it leaks,
  publish a warning and a manually installed release with a new key.

## Steps

1. Set the version in `runner/Cargo.toml` (`X.Y.Z`; pre-release tags are never offered) and update
   `CHANGELOG.md`.
2. Build on the release machine (Linux x86_64; repeat on aarch64 for that asset):

   ```sh
   scripts/release.sh 0.9.0
   ```

   This writes `dist/csr-0.9.0-x86_64-linux` (the raw binary the updater downloads),
   `dist/csr-0.9.0-linux-x86_64.tar.gz` (manual installs) and `dist/SHA256SUMS`, and prints the
   signing command. It reads no key.
   **Windows beta (optional).** Before running `scripts/release.sh`: run the `windows.yml` workflow on the tag
   (GitHub Actions), download the artifact `csr-windows-x86_64` (it holds `csr.exe`), then run

   ```sh
   scripts/package-windows.sh 0.9.0 path/to/csr.exe
   ```

   which writes `dist/csr-0.9.0-x86_64-windows.zip`. Then run `scripts/release.sh 0.9.0` as usual: when that zip
   exists it is added to `SHA256SUMS`, so the offline signature covers it. Upload the zip with the other assets.
   Without the zip `release.sh` behaves as before.

3. Copy `dist/SHA256SUMS` to the offline machine and sign it:

   ```sh
   openssl pkeyutl -sign -rawin -inkey csr-release.key -in SHA256SUMS -out SHA256SUMS.sig
   ```

   `SHA256SUMS.sig` holds the 64 raw signature bytes (128 hex characters are accepted too). Check it:

   ```sh
   openssl pkey -in csr-release.key -pubout -out csr-release.pub
   openssl pkeyutl -verify -rawin -pubin -inkey csr-release.pub -in SHA256SUMS -sigfile SHA256SUMS.sig
   ```

4. Create the GitHub release with tag `v0.9.0` (not a pre-release, marked "latest") and upload
   `csr-0.9.0-x86_64-linux`, the tarball, `SHA256SUMS` and `SHA256SUMS.sig`. All asset lines must be
   in the one signed `SHA256SUMS`.
5. Check from a running node: `csr update check` shows `newer: true`.

The release that switches the mainnet gate on (D5 B) is made the same way after the figures have
been checked by a person.
