# Roadmap

Testnet always comes first. Each phase runs on testnet before mainnet.

## Current phase: Phase 2 – Service Runtime on testnet (A8b polish done)

## Next runner release: 0.5.4 (planned, not scheduled)

- [x] Dashboard: show the **Runner ID** (the ID the CSR Network Monitor lists, a hash of the runner's service key)
      on the Overview and Network pages, with a link to the runner's entry on the monitor. Today an operator cannot
      match their node to a row of the public ranking. Shipped in 0.6.0 (with the service catalog, no separate 0.5.4 release).
- Monitor side, no runner release needed (csr-network-monitor D45/D46, Nov 2026): runners without an ok probe for
  3 days are shown as inactive (no rank, not in the gate) and hidden after 30 days; several runners with one peer ID
  count as one node.

## Runner release 0.6.0: on-chain service catalog (planned, spec ready)

- [ ] New type script `csr-service-manifest`: a developer publishes one manifest cell per service (name, version,
      description, links, code and data cells, args). Only the lock that owns the code cell can publish its
      manifest. The existing contracts do not change.
- [ ] Composite services (program + data cells + args, e.g. JavaScript on ckb-js-vm), identified by a service ID
      (equal to the code hash for single-binary services, so existing job cells keep working).
- [ ] The runner reads all manifests from its own node once a day (no monitor), lists them on the Services page
      under "Available on chain" with **Add** and **Add & approve**, lists service IDs that open job cells use
      without a manifest, and shows "New service available" under *Service runtime* on the Overview. Nothing runs
      until the admin approves it.
- [ ] `csr service publish` for app developers, and `docs/publishing-a-service.md`.
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
- [ ] Host-interface RFC in nervosnetwork/rfcs
- [ ] On-chain service catalog and composite services (0.6.0, see above)
- [ ] First service: CRT (CKB Revocable Timelock)
