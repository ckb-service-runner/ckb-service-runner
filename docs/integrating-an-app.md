# Integrating an app with CSR

For app developers. CSR lets your app's own transactions leave a **job cell** behind; independent CKB full nodes
running the CSR runner pick it up at the right time and run your **keeper service**, which builds a transaction
skeleton the runner signs and submits. Worked example: CrowdCell auto-settle (a pattern, not a CrowdCell decision).

## What CSR gives you, and what it does not

Gives you:
- Keeper jobs (at a block, epoch or time, or on demand) run by independent operators, paid by a tip you set.
- No change to your own contracts: the job is one extra output in a transaction you already build.

Does not give you:
- **No on-chain runner assignment.** Any registered runner may run the job; none is obliged to.
- **Trust-minimised, not trustless.** The job lock enforces the tip, the fee cap and the refund; it cannot make a
  runner show up. Your own locks decide where funds go, so a runner can never redirect them.
- **Runners must approve the service.** A runner runs nothing until its admin approves your service (the manifest
  makes it appear in their list, see `publishing-a-service.md`). If nobody approves, the job is never run.

## Steps

1. **Write the keeper service.** It runs in CKB-VM against the host interface (`spec/host-interface.md`). Read cells
   with `cells_query`, then hand back the settlement transaction with `tx_skeleton_put`. The skeleton must not spend
   runner, job or registry cells; the runner adds its own inputs and the job input. If there is nothing to do, put
   an **empty skeleton**. A service must tolerate being run more than once for one job cell.
2. **Deploy the code (and data) cells** with the lock that will own the manifest.
3. **Publish the manifest** so runners can find the service: `csr service publish --code-outpoint 0x<tx>:0 …`
   (see `docs/publishing-a-service.md`). It prints the service ID.
4. **Add job cells to your app's transactions** with `sdk/js/csr-job.mjs` (`jobCell`, `jobCellDeps`,
   `watchPayload`, `minJobCapacity`; see `sdk/js/README.md`):

```js
import * as csr from './csr-job.mjs';
const deployment = csr.loadDeployment(await (await fetch('/deployments/mainnet.json')).json());
const { cellOutput, outputData } = csr.jobCell({
  deployment, serviceId: SERVICE_ID,
  job: { trigger: { kind: 'AtBlock', block: deadline + MARGIN }, tipShannons: TIP, maxFeeShannons: MAX_FEE,
         maxCycles: 70_000_000n, payload: csr.watchPayload({ txHash, index }, '0x'),
         expiryBlock: deadline + EXPIRY, refundLockHash: userLock.hash(), remainingRuns: 1 },
});
tx.addOutput(cellOutput, outputData);
tx.addCellDeps(csr.jobCellDeps(deployment));
```

Show it as an optional **"settle automatically"** checkbox: ticked, add the job output; unticked, the transaction
is the one you had before.

## Worked example: CrowdCell auto-settle

- **create** with the box ticked also creates a job `AtBlock(deadline)` that **finalizes** the campaign.
- **back** with the box ticked also creates a job `AtBlock(deadline + margin)` that **settles that backer's pledge**
  (release or refund). CrowdCell's locks still decide where funds go.
- **One backer per transaction, so one job per pledge.** One job cell per campaign (`AtBlock` with
  `remaining_runs > 1`) would need contracts v4 and is not possible now.
- **Finalization before settlement:** the margin keeps the settle job behind the finalize job; if finalization has
  not happened yet, the settle job's skeleton can do it first or return an empty skeleton to be retried (job
  failures count, see below, so prefer a margin that is large enough).
- **Already done:** if someone else settled first, the keeper returns an **empty skeleton**; the job then settles
  with only its tip and the rest returns to `refund_lock_hash`.
- **Costs.** The job cell parks capacity ≈ occupied capacity + tip + max fee (`minJobCapacity`). Everything but the
  tip and the fee returns to `refund_lock_hash` when the job is consumed. The user pays tip + fee even if the bot
  settled first, unless the app decides otherwise (open question for CrowdCell).
- **The 1 CKB `MAX_FEE` slack in CrowdCell's locks** must cover the fee of the settlement transaction; keep
  `max_fee_shannons` consistent with it, or the lock rejects the skeleton.

## Failure modes

- **No runner approved the service:** the job is never run. After `expiry_block` the owner of `refund_lock_hash`
  can reclaim; without that the capacity stays parked.
- **Refund after `expiry_block`:** only a transaction with an input whose lock hash is `refund_lock_hash`, and
  whose job input has `since` an absolute block number ≥ `expiry_block`, can spend the job cell (the run path is
  not needed). The app or user must reclaim it. With the SDK, rebuild the cell deps with `jobCellDeps(deployment)`
  (no registry dep), spend the job cell with `since = expiryBlock` (absolute block, `0x0000000000000000` flags
  with the value), add an input from the user's wallet (lock hash = `refund_lock_hash`) and an output to the user.
  A "reclaim" button in the app is a good place for this.
- **Runner race:** several runners may try the same job. One transaction wins; the others fail at submit and pay
  nothing.
- **Service bugs:** a job whose run fails is retried; after **3 failures runners skip it**, so test the service on
  devnet (`docs/testing.md`) and make it idempotent.
- **Runs after expiry** stay possible until somebody refunds; a script can only lower-bound time.

## Parameters your app chooses

| Parameter | Guidance |
|---|---|
| `tipShannons` | What a runner earns; too low and nobody picks the job. Show it to the user. |
| `maxFeeShannons` | A realistic fee for the settlement transaction (F6), not a guess at zero; too low makes runs fail. |
| `maxCycles` | Upper bound for the keeper's run; measure on devnet and add headroom. |
| margin | Blocks after the deadline for the settle job (CrowdCell decides). |
| `expiryBlock` | Later than the trigger by enough runs and retries; after it the owner can refund. |
| `remainingRuns` | `1` for one-shot jobs. |
