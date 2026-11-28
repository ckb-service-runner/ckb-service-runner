# Integrating an app with CSR

For app developers. CSR lets your app's own transactions leave a **job cell** behind; independent CKB full nodes
running the CSR runner pick it up at the right time and run your **keeper service**, which builds a transaction
skeleton the runner signs and submits. Worked example: CrowdCell auto-finalize.

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

## Worked example: CrowdCell auto-finalize

Creator side only (release 0.7.1). A campaign must be **finalized** (`Active` -> `Success` or `Failed`) after its
deadline before pledges can be released or refunded; today a keeper bot does it.

- **create** with the box "finalize automatically" ticked also creates one job cell `AtBlock(deadline)` for the
  service **`crowdcell-finalize`**. Payload: `crowdcellFinalizePayload({ typeScript, campaignCodeOutPoint,
  lockCodeOutPoint })` = the campaign type script, plus the out points of the code cells of the `campaign` type and
  the `campaign-lock` lock (cell deps of the finalization). Example: `sdk/js/examples/crowdcell-create-with-finalize.mjs`.
- **What the run does:** it finds the live campaign cell by its type script and builds the transaction CrowdCell's
  contracts accept: one input with `since` = deadline, one identical output except the status byte (`Success` if
  `total_pledged >= funding_goal`, else `Failed`). The campaign keeps every shannon; the fee and the tip come from
  the job cell.
- **Already finalized (or no live campaign cell):** the service returns an **empty skeleton**. The run is an empty
  run: it settles with only its tip and the rest of the job cell returns to `refund_lock_hash` (D54). The tip cannot
  be waived; a keeper that finalized first therefore costs the creator the tip.
- **No cleanup job.** Since CrowdCell's contracts of 31 Oct, campaign destruction refuses any other typed input
  (error 27), and a CSR run always consumes the typed job cell, so a cleanup job cannot work.
- **Parked:** backer-side settle jobs (one job per pledge, created together with the pledge) and the payback receipt
  lock are not part of this release.
- **Fee note:** the "1 CKB `MAX_FEE` slack" in CrowdCell's locks applies to release and refund only. It does not
  apply to finalize: there fee and tip come from the job cell, so `maxFeeShannons` just has to cover the fee.
- **The keeper stays the fallback** and should hold back for a short margin on campaigns that have a job cell, so
  that the runner normally finalizes first and empty runs stay rare.
- **When to show the option:** hide it for campaigns with early finalization, and on mainnet until the runtime is
  live (monitor field `runtime_unlocked`, to come).
- **Expiry and reclaim:** set `expiryBlock` a few days after the deadline. After `expiry_block` the creator can
  reclaim the job cell (see "Refund after `expiry_block`" below) with `reclaimJob({ deployment, jobOutPoint,
  expiryBlock })`, which returns `{ cellDeps, jobInput }`.
- **Costs.** The job cell parks capacity ≈ occupied capacity + tip + max fee (`minJobCapacity`); all but tip and fee
  returns to `refund_lock_hash` when it is consumed.

## Failure modes

- **No runner approved the service:** the job is never run. After `expiry_block` the owner of `refund_lock_hash`
  can reclaim; without that the capacity stays parked.
- **Refund after `expiry_block`:** only a transaction with an input whose lock hash is `refund_lock_hash`, and
  whose job input has `since` an absolute block number ≥ `expiry_block`, can spend the job cell (the run path is
  not needed). The app or user must reclaim it. With the SDK, rebuild the cell deps with `jobCellDeps(deployment)`
  (no registry dep; `reclaimJob` returns them with the job input), spend the job cell with `since = expiryBlock` (absolute block, `0x0000000000000000` flags
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
| `expiryBlock` | Later than the trigger by enough runs and retries; after it the owner can refund. |
| `remainingRuns` | `1` for one-shot jobs. |
