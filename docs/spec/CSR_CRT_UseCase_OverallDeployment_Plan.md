# CSR / CRT / Use Case — Overall Deployment Plan

**Status:** draft v0.1 — 2026-10-18
**Owner:** Patrick (psawyerberlin)
**Repos:** `ckb-service-runner` (CSR + Network Monitor), `ckb-revocable-timelock` (CRT), dead man switch (to be created)

---

## 1. Goal

Build, in order, on **testnet first**:

1. **CSR — CKB Service Runner:** a side application beside a fully synced CKB full node (or Neuron in full-node mode). It provides a node dashboard and runs any service (RISC-V code stored in a cell), timed or on demand, for tips.
2. **CSR Network Monitor:** a central service that checks all CSR nodes, ranks them and publishes the statistics.
3. **CRT — CKB Revocable Timelock:** the first real service on CSR.
4. **Dead man switch:** a simple application using CRT.

Only after all four work end to end do we judge whether the concept holds.

---

## 2. Components

| # | Component | What it is | Main artifacts |
|---|---|---|---|
| A | **CSR** | Rust binary beside the node: dashboard, R2 sync guard, ping responder, runtime (CKB-VM), job execution, float management | `csr` binary, embedded dashboard, config, host interface spec |
| B | **CSR Network Monitor** | Central server on a VPS with fixed IP, own domain and public API | monitor service, public website, JSON API, attestation key, weekly stat cell |
| C | **On-chain scripts (CSR)** | Registry cell, float lock, job cell standard | Rust/ckb-std scripts, `csr-dev` CLI |
| D | **CRT** | CRT code cell + switch scripts + client SDK | service ELF, switch type script, TypeScript SDK (CCC), test page |
| E | **Dead man switch** | Web app using CRT | web app (TS + CCC + JoyID), recipient page |
| F | **Shared** | Specs and test tooling | job cell standard, host interface, CRT protocol, devnet harness (OffCKB + N runners), threat model |

---

## 3. Key design decisions

### 3.1 Node registration
- Registration cell is **funded from the operator's own wallet** (Neuron, JoyID, multisig). CSR prepares the unsigned transaction; the operator signs it.
- **Only full nodes may register.** Two layers:
  - CSR refuses to prepare a registration unless its local R2 checks pass (convenience only).
  - The registry script accepts a registration only with an **attestation signed by the Network Monitor** after it has verified the node.
- Testnet: one monitor key attests. Later: several independent probes, threshold of signatures.

### 3.2 Reachability and ping
| Node type | How it is checked |
|---|---|
| **Publicly reachable** | Monitor pings the CSR ping port (challenge-response) and checks the CKB P2P port by requesting blocks at random heights |
| **Behind NAT** | Operator enables "initiate ping": CSR connects to the monitor, the monitor sends a challenge, CSR answers |

- Challenge = nonce + random block height. Answer = tip hash, block hash at that height, version, uptime, signed with the CSR service key.
- Known limit: NAT nodes cannot be checked over P2P, so their proof is weaker (a script reading someone else's RPC could answer). The ranking shows the check type, and applications such as CRT can require reachable nodes.

### 3.3 Operator float cell
- The operator funds a **float cell**. CSR uses it to pay transaction fees; each job pays the fee back plus a tip, so the float grows. The operator withdraws at any time.
- Float lock has two paths:
  - **Runner (service key):** may only spend the float if it returns to the same float lock, minus at most the **max fee per transaction**. The key cannot move funds elsewhere.
  - **Owner (operator's wallet):** withdraw any amount, change settings, close the float.
- **Max fee per transaction:** set by the owner when creating the float, with a default value proposed by CSR.
- Theft of the service key can only burn fees, capped per transaction.

### 3.4 Tips and job selection
- **Tip size is set by the consumer** (e.g. CRT, CrowdCell automation) in the job cell.
- CSR setting: **minimum CKB per CPU** (per million cycles). Jobs below it are ignored.
- When there are more jobs than capacity, CSR **picks the best-paid jobs first**.

### 3.5 Job kinds
| Kind | Example | Runners needed |
|---|---|---|
| **Keeper** | finalize a CrowdCell campaign, run a transaction every x days | one (first valid wins) |
| **Custodian** | CRT: hold a key share, delete on cancel, release after deadline | many (threshold) |

### 3.6 Service code
- Any language that compiles to RISC-V; JS via ckb-js-vm. Supported in v1: **Rust, C, JS**.
- Service code lives in a **code cell**; jobs refer to it by code hash. The CRT code cell is deployed once; each dead man switch creates a switch cell referring to it.
- Host interface = CKB script syscalls + service calls (storage, sign, prekey decrypt, delete-on-consume). No network, no filesystem.

### 3.7 CRT via the chain only
- Each runner publishes one-time public keys (**prekeys**) in its registry cell.
- The user splits the secret and encrypts each share to a node's prekey, stored in the switch cell.
- Cancel → nodes delete that prekey → share unreadable forever.
- Release after the deadline → nodes publish their decrypted shares on-chain.
- Owner can **update the deadline** without re-sharing (dead man switch renewal).
- Node selection from the Network Monitor ranking (availability, reachability, country/ASN diversity).
- v1 limit: the user (dealer) knows the secret.

---

## 4. Build sequence

| Step | Build | Depends on | Exit criteria |
|---|---|---|---|
| **A1** | CSR dashboard & control: node status, R2 guard, resources, settings, ping responder (inbound + NAT-initiated), "Join ranking" toggle | — | runs a week beside testnet `ckb` and Neuron without problems |
| **B1** | Network Monitor: ping/challenge, P2P full-node check, GeoIP (country/ASN, no IPs published), ranking, public site + API | A1 | 10+ runners ranked correctly; fake (RPC-proxy) node fails the P2P check; replayed answer fails |
| **A1+** | CSR "Network" page: worldwide node count, map, own ranking and availability (from monitor API) | B1 | figures match the monitor |
| **A2** | CSR runtime core: CKB-VM sandbox, host interface, encrypted storage, local services | A1 | same service in Rust, C and JS gives identical results; limits enforced |
| **B2** | Monitor attestation signer + weekly stat cell | B1 | attestation accepted by registry script on devnet |
| **A3** | CSR on-chain: registry cell, float lock, job cell standard, keeper jobs, tips, min CKB/CPU selection | A2, B2 | registration rejected without attestation; float cannot be drained to another address; float grows by tip − fee; CrowdCell campaign finalized by CSR; heartbeat job runs for days |
| **D** | CRT: code cell, switch scripts, prekeys, node selection, SDK, test page | A3 | 5-of-10 on devnet, then testnet; early release rejected; cancel deletes shares |
| **E** | Dead man switch web app | D | testnet runs with short deadlines: renew, missed renewal → release, recipient decrypts |

---

## 5. Deployment phases

| Phase | Period | On the network | Purpose |
|---|---|---|---|
| **0 — Build** | now → first release | testnet only | build A1, B1, A1+ |
| **1 — Dashboard only** | first **6 months** after release | **CSR + CSR Network Monitor only** (testnet and mainnet nodes) | operators install CSR for the node dashboard and ranking; grow the node count and history |
| **1 (parallel, testnet)** | same 6 months | A2, A3, CRT, dead man switch on testnet | build and test the runtime and use cases without affecting mainnet operators |
| **2 — Services** | after 6 months **and** enough diverse nodes | CRT live; runtime enabled | first real service and first use case |

- During Phase 1 the runtime toggle on mainnet is visible but inactive.
- "Enough nodes" threshold to confirm (earlier roadmap: 60 nodes diverse by ASN, region, country).

---

## 6. Testing infrastructure

- **Devnet harness:** OffCKB + N CSR instances + a local monitor, started with one command. Debug-only mode lets several runners share one devnet node.
- **Testnet:** own nodes plus volunteer operators.
- **Exploit tests** for every on-chain script (registry, float, job cells, CRT switch).
- No mainnet transaction in any automated test.

---

## 7. Open points

1. Node threshold and diversity rule for Phase 2.
2. Default value for max fee per transaction in the float.
3. Monitor domain name and VPS.
4. Where the dead man switch stores larger files (cells vs external storage).
5. Decentralising the monitor (several probes, threshold attestations): when?
6. Should CRT accept NAT-only nodes, or require publicly reachable ones?
