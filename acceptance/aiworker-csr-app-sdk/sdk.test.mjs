// csr-app-sdk: fixed vectors, written by the spec author.
// blake2b vectors computed with hashlib.blake2b(..., digest_size=32, person=b"ckb-default-hash").
import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const sdk = await import(pathToFileURL(path.join(process.cwd(), 'sdk', 'js', 'csr-job.mjs')).href);
const hex = (b, n) => '0x' + b.repeat(n);

test('service ID of a single binary is its code hash', () => {
  assert.equal(sdk.serviceId(hex('11', 32)), hex('11', 32));
});

test('service ID of composite services (D49)', () => {
  assert.equal(sdk.serviceId(hex('11', 32), '0xaabb', [hex('22', 32)]),
    '0xe8e694810480390458e45747bb05fd964cb016f2653f0b6808beb3409d88080a');
  assert.equal(sdk.serviceId(hex('11', 32), '0x', [hex('22', 32)]),
    '0xac675653b016d0a1174ee5d071a464af00bf4cb61decbb33481a820afb35225d');
});

test('encode/decode round trip for every trigger', () => {
  const triggers = [{ kind: 'OnDemand' }, { kind: 'AtBlock', block: 123n }, { kind: 'AtEpoch', epoch: 7n },
    { kind: 'AtTimestamp', timestampMs: 1794261707299n }, { kind: 'Every', blocks: 20n }];
  for (const trigger of triggers) {
    const job = { trigger, tipShannons: 200000000n, maxFeeShannons: 100000n, maxCycles: 70000000n,
      payload: '0x0102', expiryBlock: 23000000n, refundLockHash: hex('ab', 32),
      remainingRuns: trigger.kind === 'Every' ? 5 : 1 };
    assert.deepEqual(sdk.decodeJob(sdk.encodeJob(job)), job);
  }
});

test('minimum capacity follows the creation rule', () => {
  const deployment = { scripts: { job_type: { code_hash: hex('05', 32), hash_type: 'data2', tx_hash: hex('00', 32), index: 0 },
    job_lock: { code_hash: hex('74', 32), hash_type: 'data2', tx_hash: hex('71', 32), index: 0 } },
    registry_type_hash: hex('cb', 32) };
  const job = { trigger: { kind: 'AtBlock', block: 1n }, tipShannons: 200000000n, maxFeeShannons: 100000n,
    maxCycles: 70000000n, payload: '0x', expiryBlock: 2n, refundLockHash: hex('ab', 32), remainingRuns: 1 };
  const dataLen = BigInt((sdk.encodeJob(job).length - 2) / 2);
  const occupied = (8n + 65n + 65n + dataLen) * 100000000n;
  assert.equal(sdk.minJobCapacity({ deployment, serviceId: hex('be', 32), job }), occupied + 200100000n);
  const c = sdk.jobCell({ deployment, serviceId: hex('be', 32), job });
  assert.equal(c.cellOutput.type.args, hex('be', 32));
  assert.equal(c.cellOutput.lock.args, hex('cb', 32));
  assert.equal(c.cellOutput.lock.hashType, 'data2');
  assert.equal(BigInt(c.cellOutput.capacity), occupied + 200100000n);
});

test('watch payload header', () => {
  const p = sdk.watchPayload({ txHash: hex('ab', 32), index: 7 }, '0xff');
  assert.ok(p.startsWith('0x43535257' + '01' + 'ab'.repeat(32) + '07000000'));
  assert.ok(p.endsWith('ff'));
});
