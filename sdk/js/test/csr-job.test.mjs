import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import * as sdk from '../csr-job.mjs';

const hex = (b, n) => '0x' + b.repeat(n);
const deployment = JSON.parse(readFileSync(new URL('../../../deployments/testnet.json', import.meta.url), 'utf8'));
const job = { trigger: { kind: 'AtBlock', block: 1n }, tipShannons: 200000000n, maxFeeShannons: 100000n,
  maxCycles: 70000000n, payload: '0x', expiryBlock: 2n, refundLockHash: hex('ab', 32), remainingRuns: 1 };

const big = (o) => ({ ...o, trigger: Object.fromEntries(Object.entries(o.trigger).map(([k, v]) => [k, k === 'kind' ? v : BigInt(v)])),
  tipShannons: BigInt(o.tipShannons), maxFeeShannons: BigInt(o.maxFeeShannons), maxCycles: BigInt(o.maxCycles), expiryBlock: BigInt(o.expiryBlock) });

test('vectors.json matches the module', () => {
  const vectors = JSON.parse(readFileSync(new URL('./vectors.json', import.meta.url), 'utf8'));
  assert.ok(vectors.length >= 6);
  for (const v of vectors) {
    assert.equal(sdk.encodeJob(big(v.job)), v.encoded);
    assert.deepEqual(sdk.decodeJob(v.encoded), big(v.job));
  }
});

test('round trip for every trigger kind', () => {
  for (const trigger of [{ kind: 'OnDemand' }, { kind: 'AtBlock', block: 123n }, { kind: 'AtEpoch', epoch: 7n },
    { kind: 'AtTimestamp', timestampMs: 1794261707299n }, { kind: 'Every', blocks: 20n }]) {
    const j = { ...job, trigger, payload: '0x0102', remainingRuns: trigger.kind === 'Every' ? 5 : 1 };
    assert.deepEqual(sdk.decodeJob(sdk.encodeJob(j)), j);
  }
});

test('decode rejects malformed data', () => {
  const h = sdk.encodeJob(job);
  assert.throws(() => sdk.decodeJob(h + '00'));
  assert.throws(() => sdk.decodeJob(h.slice(0, -2)));
  assert.throws(() => sdk.encodeJob({ ...job, tipShannons: -1n }));
});

test('capacity and job cell', () => {
  const dataLen = BigInt((sdk.encodeJob(job).length - 2) / 2);
  const min = (8n + 65n + 65n + dataLen) * 100000000n + 200100000n;
  const sid = hex('be', 32);
  assert.equal(sdk.minJobCapacity({ deployment, serviceId: sid, job }), min);
  const five = { ...job, remainingRuns: 5 };
  assert.equal(sdk.minJobCapacity({ deployment, serviceId: sid, job: five }), min + 4n * 200100000n);
  const c = sdk.jobCell({ deployment, serviceId: sid, job, extraCapacityShannons: 5n });
  assert.equal(BigInt(c.cellOutput.capacity), min + 5n);
  assert.equal(c.cellOutput.type.args, sid);
  assert.equal(c.cellOutput.lock.args, deployment.registry_type_hash);
  assert.equal(c.cellOutput.lock.codeHash, deployment.scripts.job_lock.code_hash);
  assert.equal(c.outputData, sdk.encodeJob(job));
});

test('cell deps and deployment check', () => {
  const deps = sdk.jobCellDeps(deployment);
  assert.deepEqual(deps[0], { outPoint: { txHash: deployment.scripts.job_type.tx_hash, index: 0 }, depType: 'code' });
  assert.equal(deps.length, 2);
  assert.throws(() => sdk.loadDeployment({ scripts: {} }));
});

test('watch payload header', () => {
  const p = sdk.watchPayload({ txHash: hex('ab', 32), index: 7 }, '0xff');
  assert.equal(p, '0x43535257' + '01' + 'ab'.repeat(32) + '07000000' + 'ff');
});

test('blake2b-256 with ckb personalization', () => {
  const h = (b) => '0x' + [...sdk.blake2b256(b)].map((x) => x.toString(16).padStart(2, '0')).join('');
  assert.equal(h(new Uint8Array(0)), '0x44f4c69744d5f8c55d642062949dcae49bc4e7ef43d388c5a12f42b5633d163e');
  assert.equal(h(new TextEncoder().encode('abc')).length, 66);
  // multi-block input must not throw and must differ from a one-byte change
  assert.notEqual(h(new Uint8Array(300)), h(new Uint8Array(300).fill(1, 299)));
});

test('service IDs (D49)', () => {
  assert.equal(sdk.serviceId(hex('11', 32)), hex('11', 32));
  assert.equal(sdk.serviceId(hex('11', 32), '0xaabb', [hex('22', 32)]),
    '0xe8e694810480390458e45747bb05fd964cb016f2653f0b6808beb3409d88080a');
  assert.equal(sdk.serviceId(hex('11', 32), '0x', [hex('22', 32)]),
    '0xac675653b016d0a1174ee5d071a464af00bf4cb61decbb33481a820afb35225d');
});
