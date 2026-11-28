# Security model

For operators and reviewers. It describes what the code does in release 0.7; where something is not done, it says so
under [Known gaps](#known-gaps). Report vulnerabilities as described in [`SECURITY.md`](../SECURITY.md).

## What CSR reads

- **The CKB node's RPC**, on loopback only (`127.0.0.1`): chain tip, sync state, peers, the node ID. CSR does not
  start or configure the node.
- **The node's P2P key file**, the path set in `node.p2p_key_file` (the node's `secret_key`). It is read only while
  CSR signs the node-identity proof: the file is read again for each proof, the key is checked against the peer ID
  stored at start-up and against the node ID the node reports, used to sign, and erased from memory immediately. Between
  proofs CSR holds only the file path and the derived peer ID. The key is never logged and never sent.
  If the key file is missing, unreadable or holds another key, no proof is signed.
- **Cells from its own node, for Verify binary.** The dashboard's **Verify binary** button (`GET /api/services/{id}/verify`,
  read-only) makes the runner read a service's code cell and data cells from the operator's **own node** only, hash
  them and recompute the service ID. It checks the on-chain bytes against the service ID, not the publisher's source:
  a source link in a manifest is the publisher's claim, not a proof. The **explorer links** in the detail panel are
  plain links the browser opens when the operator clicks; the runner itself never contacts the explorer or any other
  site.
- **Its own files** in the CSR data root (`/var/lib/csr` on Linux, `%ProgramData%\csr` on Windows): configuration,
  its database, the runner's own keys.

## What CSR never touches

- Wallet keystores, including Neuron's wallet files, and any other wallet of the user.
- The node's `ckb.toml` and the node's data directory (chain data, indexer, peer store). The only file of the node CSR
  opens is the P2P key file named above, read-only.

The runner wallet key (used for staking and job transactions) lives in the CSR data root. It belongs to CSR; it is not
a user wallet and is not imported from one.

## What leaves the machine

- **The signed node-identity proof**: peer ID, service public key, network, timestamp, the node's public key and two
  signatures (one by the node key, one by the service key). The node's secret key is never part of it.
- **Ping and check-in statistics** sent to the CSR Network Monitor (reachability, latency, node tip and sync state).
- **The approved service IDs** (sent with the check-in, only when the ranking is joined): the sorted list of the
  service IDs that are approved and enabled on this runner and allowed for this network, at most 64. It is advisory
  and not signed (the check-in signature does not cover it), so the monitor can count how many runners will run a
  service. It reveals which services an operator runs; no names, arguments or outpoints are sent. It is empty on
  mainnet. You can see the same list on the Services page of the dashboard (the approved, enabled services).
- **What the Service Runtime publishes on chain** (job settlements, registrations), which is public by nature.
- According to the monitor's design, the CSR Network Monitor stores no IP address of runners. That is the monitor's
  published design and is not something the runner can enforce or prove.

The updater also contacts GitHub releases to fetch binaries and the signed `SHA256SUMS`.

## Privileges by platform

**Linux.** CSR runs as the system user `csr` (unit `config/csr@.service`, one instance per network) with these
systemd options: `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`,
`ReadWritePaths=/var/lib/csr`, `MemoryMax=256M`, `LimitCORE=0`, plus `Nice=19` and idle I/O and CPU scheduling. The
node's key file is made readable for the `csr` user through an ACL or a group (see `docs/install.md`), not by
changing its owner.

**Windows (0.7).**
- One native Windows service per network, `CSR-<network>`, under the virtual account `NT SERVICE\CSR-<network>`.
  No password, not SYSTEM; the service holds only `SeChangeNotifyPrivilege`.
- The data root `%ProgramData%\csr` has inheritance removed; full control for Administrators and for the service
  accounts only. SYSTEM is not granted.
- The node's P2P key file gets exactly one extra ACE: read-only for that network's service account. The uninstaller
  removes that ACE before it deletes the service and touches nothing else in the node's folders.
- **On Windows the node key is readable by the service account for as long as the installation lasts.**
- The installer searches for key files only in `-CkbDir`, the folders of running `ckb.exe` processes and
  `%APPDATA%\Neuron`; a wider search needs `-SearchWide` and a yes. Each candidate is printed as `key candidate: <path>`.
- Firewall: in `outbound` ping mode (the installer writes it into new configurations; the runner's own default is `inbound`) the runner binds no ping listener and no rule is created (an old
  rule is removed). The inbound rule exists only when `ranking.ping_mode` is `inbound` or `both`.
- The dashboard listens on `127.0.0.1` only and is never opened in the firewall.

## Verifying a download

Two independent checks:

1. **`SHA256SUMS` signature.** Each release carries `SHA256SUMS` and `SHA256SUMS.sig`, an ed25519 signature made
   offline with the release key (never in the repository or in CI). The public key is pinned in the runner
   (`RELEASE_PUBKEY`) and published in `docs/release.md`; the updater verifies it. It proves the file list was approved
   by the holder of the release key.
2. **Build provenance.** Release binaries are built by GitHub Actions from the tag and carry an attestation:
   `gh attestation verify <file> --repo ckb-service-runner/ckb-service-runner`. It proves the file was built by that
   workflow from this repository at that commit. It does not say the source is free of bugs.

Use both: the signature ties a file to the release key, the attestation ties it to the public source.

## Known gaps

- No Authenticode signature yet. It is planned and depends on a code-signing provider; Windows SmartScreen may warn.
- No independent security audit yet. It is planned before the mainnet Service Runtime.
- Testnet and mainnet services of one machine share one data root (as on Linux).
- The monitor is trusted for ranking; CSR cannot verify its ranking.
- A compromised service account can read the node key for as long as CSR is installed.
- CSR does not tighten the permissions of the node's own files. A node folder created in the root of a drive (for
  example `C:\ckb\data`) usually inherits `Users` and `Authenticated Users` entries, so every account on the PC can
  already read the key file, and `Authenticated Users` can even change it. The read entry that the installer adds is
  then redundant. Keep the node's data folder in a location with restricted permissions.
- Mainnet runs as a node dashboard only; the Service Runtime stays off on mainnet.
