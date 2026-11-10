// Regenerates vectors.json: node sdk/js/test/gen-vectors.mjs
import { writeFileSync } from 'node:fs';
import { encodeJob } from '../csr-job.mjs';

const refund = '0x' + 'ab'.repeat(32);
const base = { tipShannons: 200000000n, maxFeeShannons: 100000n, maxCycles: 70000000n, payload: '0x', expiryBlock: 23000000n, refundLockHash: refund, remainingRuns: 1 };
const jobs = [
  { ...base, trigger: { kind: 'OnDemand' } },
  { ...base, trigger: { kind: 'AtBlock', block: 123n }, payload: '0x0102' },
  { ...base, trigger: { kind: 'AtEpoch', epoch: 7n } },
  { ...base, trigger: { kind: 'AtTimestamp', timestampMs: 1794261707299n }, payload: '0x' + '5a'.repeat(100) },
  { ...base, trigger: { kind: 'Every', blocks: 20n }, remainingRuns: 5 },
  { ...base, trigger: { kind: 'AtBlock', block: 18446744073709551615n }, tipShannons: 0n, expiryBlock: 18446744073709551615n, payload: '0xff' },
];
const str = (j) => JSON.parse(JSON.stringify(j, (_, v) => (typeof v === 'bigint' ? v.toString() : v)));
writeFileSync(new URL('./vectors.json', import.meta.url), JSON.stringify(jobs.map((job) => ({ job: str(job), encoded: encodeJob(job) })), null, 2) + '\n');
