// Example (documentation, not run in tests): CrowdCell's "create campaign" transaction with the optional
// "finalize automatically" job cell. Frontend: Next.js + CCC (@ckb-ccc/core).
import { ccc } from '@ckb-ccc/core';
import * as csr from '../csr-job.mjs';

/**
 * @param {ccc.Transaction} tx       the create-campaign transaction CrowdCell already builds
 * @param {ccc.Signer} signer        the creator's wallet
 * @param {object} p
 *   deployment                CSR deployment JSON (deployments/<network>.json)
 *   finalizeServiceId         service ID printed by `csr service publish`
 *   campaignTypeScript        the campaign cell's type script (ccc.Script)
 *   campaignCodeOutPoint / lockCodeOutPoint   code cells of the campaign type and campaign-lock scripts
 *   deadlineBlock             bigint, the campaign deadline
 *   tipShannons, maxFeeShannons               parameters: show them to the user
 */
export async function addFinalizeJob(tx, signer, p) {
  const { script: userLock } = await signer.getRecommendedAddressObj();

  const job = {
    trigger: { kind: 'AtBlock', block: p.deadlineBlock },
    tipShannons: p.tipShannons,           // what the runner earns
    maxFeeShannons: p.maxFeeShannons,     // cap for the finalize transaction fee (comes from the job cell)
    maxCycles: p.maxCycles ?? 70_000_000n, // measure on testnet, then adjust
    payload: csr.crowdcellFinalizePayload({
      typeScript: p.campaignTypeScript.toBytes ? ccc.hexFrom(p.campaignTypeScript.toBytes()) : p.campaignTypeScript,
      campaignCodeOutPoint: p.campaignCodeOutPoint,
      lockCodeOutPoint: p.lockCodeOutPoint,
    }),
    // a few days after the deadline (~ 8 s blocks: 3 days = 32_400 blocks); after it the creator can reclaim
    expiryBlock: p.deadlineBlock + (p.expiryAfterBlocks ?? 32_400n),
    refundLockHash: userLock.hash(),      // the rest of the job cell returns here
    remainingRuns: 1,
  };

  const { cellOutput, outputData } = csr.jobCell({ deployment: p.deployment, serviceId: p.finalizeServiceId, job });
  tx.addOutput(cellOutput, outputData);
  tx.addCellDeps(csr.jobCellDeps(p.deployment).map((d) => ({ outPoint: { txHash: d.outPoint.txHash, index: d.outPoint.index }, depType: d.depType })));

  await tx.completeInputsByCapacity(signer);
  await tx.completeFeeBy(signer);
  return tx;
}

// Only offer the option when the campaign has no early finalization and, on mainnet, once the runtime is live.
// Ticked: call addFinalizeJob(...) before signing. Unticked: send the transaction as before.
