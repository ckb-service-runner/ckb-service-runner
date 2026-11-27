# Roadmap

Testnet always comes first. Each phase runs on testnet before mainnet.

## Current phase: Phase 2 – Service Runtime on testnet (A8b polish done)

## Runner release 0.7: security hardening (in progress)

Community feedback (Telegram, Nov 2026) on the Windows beta: it runs as SYSTEM, reads the node's P2P identity key
and is not code-signed. On Linux CSR already runs as its own `csr` user with systemd sandboxing; 0.7 brings Windows
to the same level and documents the security model for every platform.

Least privilege
- [x] Windows: run CSR as a native Windows service under its own limited account (virtual account
      `NT SERVICE\CSR-<network>`) instead of a scheduled task as SYSTEM; no `RunLevel Highest`.
- [x] Windows installer: grant that account read access to the one node `secret_key` file and full control of
      `%ProgramData%\csr` only (SYSTEM removed from the data root ACL).
- [x] Installer key search: look only in known locations (`-CkbDir`, running `ckb.exe` folders, Neuron's internal
      mainnet node); a wider search only after the operator agrees; print every path read.
- [x] Node P2P key: read only while signing the node-identity proof, then zeroize; not kept in memory.
- [x] Ping: the runner binds no ping listener in outbound mode (already so); the installer opens the inbound firewall rule only for inbound modes.

Signed and verifiable releases
- [ ] Authenticode-signed `csr.exe`, `install.ps1` and `uninstall.ps1` (depends on approval by a code-signing
      provider, e.g. an open-source signing program). Until then: the offline-signed `SHA256SUMS` stays the way to
      verify a download, shown in the README and release notes.
- [x] Build provenance: release binaries built by GitHub Actions from the tag, with a provenance attestation
      (`actions/attest-build-provenance`), so anyone can check a binary came from the published source.

Transparency
- [x] `docs/security-model.md`: what CSR reads (node RPC, the node's P2P key file), what it never touches (wallet
      keystores, including Neuron's), what leaves the machine (signed proof, ping statistics; the monitor stores no
      IP address), privileges per platform, known gaps. Linked from the README and the monitor site.
- [x] `SECURITY.md` status updated: mainnet runs as a dashboard only; the Service Runtime stays off on mainnet.
- [ ] Independent security audit scheduled as a milestone before the mainnet Service Runtime (see Phase 2), with a
      funding proposal (e.g. Community Fund DAO).

## Runner release 0.5.4 (merged into 0.6.0)

- [x] Dashboard: show the **Runner ID** (the ID the CSR Network Monitor lists, a hash of the runner's service key)
      on the Overview and Network pages, with a link to the runner's entry on the monitor. Today an operator cannot
      match their node to a row of the public ranking. Shipped in 0.6.0 (with the service catalog, no separate 0.5.4 release).
- Monitor side, no runner release needed (csr-network-monitor D45/D46, Nov 2026): runners without an ok probe for
  3 days are shown as inactive (no rank, not in the gate) and hidden after 30 days; several runners with one peer ID
  count as one node.

## Runner release 0.6.1 (released)

- [x] Dashboard, Runner ID layout (owner feedback on 0.6.0): remove the copy button and the separate
      "View on monitor" link next to the Runner ID. Show the **full** Runner ID on its own line directly below
      **Tip hash** and **Node ID** (same monospace facts block), and make the ID itself the link to the runner's
      row on the monitor (`<monitor>/?runner=<id>`; with several monitors, the first one). Same on the Network
      page. No shortened form.
- [x] Host interface: implement `random_bytes` (7050), required by CRT (CKB Revocable Timelock). The host fills a
      VM buffer with `len` bytes from the operating system's CSPRNG (`len` 1–1024; `11` above 1024, `12` for 0),
      available in every mode including `csr service run-local`, cost `500 + ceil(len / 4)` cycles, the bytes never
      logged or stored by the host. It lets a CRT service create a fresh secret key per switch inside the sandbox,
      which the runner can later destroy. The number is already reserved, so no existing service changes;
      7051 `prekey_decrypt` stays reserved. Includes a `randtest` example service and tests.

## Runner release 0.6.0: on-chain service catalog (released 10 Nov 2026)

- [x] New type script `csr-service-manifest`: a developer publishes one manifest cell per service (name, version,
      description, links, code and data cells, args). Only the lock that owns the code cell can publish its
      manifest. The existing contracts do not change.
- [x] Composite services (program + data cells + args, e.g. JavaScript on ckb-js-vm), identified by a service ID
      (equal to the code hash for single-binary services, so existing job cells keep working).
- [x] The runner reads all manifests from its own node once a day (no monitor), lists them on the Services page
      under "Available on chain" with **Add** and **Add & approve**, lists service IDs that open job cells use
      without a manifest, and shows "New service available" under *Service runtime* on the Overview. Nothing runs
      until the admin approves it.
- [x] `csr service publish` for app developers, and `docs/publishing-a-service.md`.
- [x] JavaScript job builder and app integration guide (first user: CrowdCell): `sdk/js/csr-job.mjs`, `docs/integrating-an-app.md`.
- Goal: **after 0.6.0, new services need no runner update.** Publish a manifest and every runner lists it within a
  day. Only changes to the runner itself (new host calls, trigger kinds, contract versions or limits) need a
  release.

## Phase 0 — Requirements

- [x] Requirements R1–R4, non-goals and the specifications in `spec/`
- [ ] Feedback from CKB / CKB-VM maintainers on the host-interface idea
- [ ] Funding for a prototype (Spark revision or Community Fund DAO proposal)

## Phase 1 — Node Runner dashboard and ranking

- [x] Dashboard (embedded, no build step) with authentication
- [x] Runner process: local node checks (R2: synced, loopback RPC, peers, indexer)
- [x] Settings page, including joining the ranking
- [x] Ranking against the CSR Network Monitor, Network page
- [x] Ping responder (inbound, outbound, both)
- [x] Service Runtime toggle
- [x] Testnet release (Linux x86_64 tarball, systemd unit)
- [ ] Weekly stat cell: not read by the runner; owned by the monitor
- [ ] Mainnet, gated by release (see below)

## Phase 2 — Service Runtime

- [x] CKB-VM sandbox with cycle, memory and wall-time limits
- [x] Host interface (`spec/host-interface.md`) and example services (Rust, C, JS)
- [x] Persistent storage with event-driven deletion
- [x] Job cells, discovery, run transactions, tips (`spec/job-cell-standard.md`)
- [x] Registry with Nervos DAO stake and consent; runner wallet; sweep
- [x] Services, Jobs, Earnings and Wallet & registration pages
- [ ] Mainnet gate by release: each release enables the mainnet runtime only after audit findings are closed and the
      monitor unlocks it (`mainnet_runtime_unlocked`)
- [ ] Security audit before mainnet
- [ ] Job assignment from the ranking (minimum reputation and diversity per application)
- [ ] Probing decentralised onto the runners themselves
- [ ] Host-interface RFC in nervosnetwork/rfcs (draft in `docs/rfc/`)
- [x] On-chain service catalog and composite services (0.6.0, see above)
- [ ] First service: CRT (CKB Revocable Timelock); needs `random_bytes` (7050), shipped in 0.6.1
