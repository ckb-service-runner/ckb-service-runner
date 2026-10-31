# Roadmap

Testnet always comes first. Each phase runs on testnet before mainnet.

## Current phase: Phase 2 – Service Runtime on testnet (A8b polish done)

## Phase 0 — Requirements

- [x] Requirements R1–R4, non-goals and the specification (`docs/spec/CSR-SPEC-v0.2.md`)
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
- [ ] First service: CRT (CKB Revocable Timelock)
