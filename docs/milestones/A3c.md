# Milestone A3, part c – CSR runs keeper jobs from the chain

## Built
- `csr-dev deploy-service` and `csr-dev job`: put a service ELF and a job cell on the devnet.
- `csr service add-onchain --outpoint`: registers a service from its code cell (`not_approved`, then approved as in A2). Jobs run only for services that are `approved_enabled` **and** in `runtime.allowed_services`.
- Discovery of `csr-job-type` / `csr-job-lock` cells of this deployment on every block, selection by tip rate (`csr jobs plan --json`), exclusions with reasons.
- Execution in the A2 sandbox, run transaction (runner key cell, job, float + tip, recreation or refund, cell deps incl. the monitor config cell), settlement tracking: `Discovered → Queued → Scheduled → Running ⇄ Paused → Completed → Submitting → Settled | Lost`, side exits `Failed`, `Rejected`, `Expired`.
- Host calls `chain_read` (7003) and `cells_query` (7004).
- API `GET /api/jobs`, `GET /api/jobs/{id}`, `GET /api/earnings`; dashboard pages **Jobs**, **Earnings & float** (balance, tips per day, settlements, Top up / Withdraw links to the Float page) and the Overview float card.
- Demo service `heartbeat` (`services/rust/heartbeat`).

## Two runners, one job
Two CSR instances (separate `--data-root`, own ports and keys; debug builds with `network = "devnet"`) can share one devnet node. Both see the job, both build a transaction spending it. The node accepts the first into its pool and refuses the second ("Dead outpoint" / double spend). The loser ends in `Lost` (also when the winner's transaction is only pending and the cell still looks live), pays nothing, and only the winner's float grows by the tip.

## Demo on devnet
```
bash scripts/build-contracts.sh && bash scripts/build-services.sh
bash scripts/devnet.sh reset && bash scripts/devnet.sh start
node --test --test-concurrency=1 acceptance/a3c/p2.test.mjs acceptance/a3c/p3.test.mjs
bash scripts/devnet.sh stop
```
Manually: `csr-dev deploy-contracts`, create runner key cell, float and registry as in `docs/testing.md` (A3b), then
```
csr-dev deploy-service --elf services/rust/target/riscv64imac-unknown-none-elf/release/heartbeat
csr service add-onchain --outpoint 0x<tx>:0 && csr service approve <code_hash>
csr-dev job --code-hash <code_hash> --tip-ckb 5 --max-fee-shannons 100000 --max-cycles 10000000 --trigger ondemand --runs 1
csr jobs plan --json
csr run          # with runtime.enabled = true; mine blocks with scripts/devnet.sh mine 1
```
Open `#/jobs` and `#/earnings` on the dashboard.

## Testnet
The runbook is in `docs/testing.md` ("Testnet runbook"). Job execution on testnet/mainnet needs shipped script hashes; mainnet stays gated.

## Not done
Shipped testnet/mainnet script hashes; non-keeper job kinds; interrupting a VM mid-run.
