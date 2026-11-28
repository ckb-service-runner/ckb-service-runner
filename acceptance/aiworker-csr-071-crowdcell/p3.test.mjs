// csr-071-crowdcell, phase 3: JS SDK helpers for the CrowdCell finalize job, types, docs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const sdk = await import(pathToFileURL(path.join(ROOT, 'sdk', 'js', 'csr-job.mjs')).href);
const h = (b) => '0x' + b.repeat(32);

function expectedPayload(scriptHex, a, b) {
  const script = Buffer.from(scriptHex.slice(2), 'hex');
  const len = Buffer.alloc(4);
  len.writeUInt32LE(script.length);
  const op = (o) => {
    const i = Buffer.alloc(4);
    i.writeUInt32LE(o.index);
    return Buffer.concat([Buffer.from(o.txHash.slice(2), 'hex'), i]);
  };
  return '0x' + Buffer.concat([len, script, op(a), op(b)]).toString('hex');
}

test('finalize payload: length, script, campaign code out point, campaign-lock code out point', () => {
  const typeScript = '0x' + '09'.repeat(85);
  const a = { txHash: h('11'), index: 1 };
  const b = { txHash: h('22'), index: 258 };
  assert.equal(sdk.crowdcellFinalizePayload({ typeScript, campaignCodeOutPoint: a, lockCodeOutPoint: b }),
    expectedPayload(typeScript, a, b));
});

test('finalize payload: refuses bad input', () => {
  const ok = { typeScript: '0x' + '09'.repeat(10), campaignCodeOutPoint: { txHash: h('11'), index: 0 }, lockCodeOutPoint: { txHash: h('22'), index: 0 } };
  assert.doesNotThrow(() => sdk.crowdcellFinalizePayload(ok));
  assert.throws(() => sdk.crowdcellFinalizePayload({ ...ok, typeScript: '0x' }));
  assert.throws(() => sdk.crowdcellFinalizePayload({ ...ok, typeScript: '0x' + '09'.repeat(4097) }));
  assert.doesNotThrow(() => sdk.crowdcellFinalizePayload({ ...ok, typeScript: '0x' + '09'.repeat(4096) }));
  assert.throws(() => sdk.crowdcellFinalizePayload({ ...ok, typeScript: '0x123' }));
  assert.throws(() => sdk.crowdcellFinalizePayload({ ...ok, campaignCodeOutPoint: { txHash: '0x1234', index: 0 } }));
  assert.throws(() => sdk.crowdcellFinalizePayload({ ...ok, lockCodeOutPoint: { txHash: h('22'), index: -1 } }));
  assert.throws(() => sdk.crowdcellFinalizePayload({ ...ok, lockCodeOutPoint: { txHash: h('22'), index: 2 ** 32 } }));
});

test('reclaimJob: the job input with since = expiry block, and the cell deps without the registry', () => {
  const deployment = JSON.parse(fs.readFileSync(path.join(ROOT, 'acceptance', 'aiworker-csr-071-crowdcell', 'deployment.fixture.json'), 'utf8'));
  const r = sdk.reclaimJob({ deployment, jobOutPoint: { txHash: h('33'), index: 0 }, expiryBlock: 23000000n });
  assert.deepEqual(r.jobInput, { previousOutput: { txHash: h('33'), index: 0 }, since: 23000000n });
  assert.deepEqual(r.cellDeps, sdk.jobCellDeps(deployment));
  assert.equal(r.cellDeps.length, 2);
  assert.throws(() => sdk.reclaimJob({ deployment, jobOutPoint: { txHash: h('33'), index: 0 }, expiryBlock: -1n }));
  assert.throws(() => sdk.reclaimJob({ deployment, jobOutPoint: { txHash: h('33'), index: 0 }, expiryBlock: 1n << 56n }));
  assert.throws(() => sdk.reclaimJob({ deployment, jobOutPoint: { txHash: '0x12', index: 0 }, expiryBlock: 5n }));
});

test('TypeScript declarations cover the whole public API of csr-job.mjs', () => {
  const dts = read('sdk', 'js', 'csr-job.d.ts');
  const src = read('sdk', 'js', 'csr-job.mjs');
  const exported = [...src.matchAll(/^export (?:async )?function (\w+)/gm)].map((m) => m[1]);
  assert.ok(exported.includes('crowdcellFinalizePayload') && exported.includes('reclaimJob'));
  for (const name of exported) {
    assert.match(dts, new RegExp(`export function ${name}\\b`), `csr-job.d.ts declares ${name}`);
  }
  assert.match(dts, /OutPoint/);
});

test('CCC example and SDK README mention the new helpers', () => {
  const ex = read('sdk', 'js', 'examples', 'crowdcell-create-with-finalize.mjs');
  assert.match(ex, /@ckb-ccc\//);
  assert.match(ex, /jobCell\(/);
  assert.match(ex, /jobCellDeps\(/);
  assert.match(ex, /crowdcellFinalizePayload\(/);
  const readme = read('sdk', 'js', 'README.md');
  assert.match(readme, /crowdcellFinalizePayload/);
  assert.match(readme, /reclaimJob/);
});

test('integrating-an-app.md states the agreed CrowdCell design', () => {
  const d = read('docs', 'integrating-an-app.md');
  assert.match(d, /crowdcell-finalize/);
  assert.match(d, /creator side/i);
  assert.match(d, /empty skeleton/i);
  assert.match(d, /no cleanup job/i);
  assert.match(d, /error 27/);
  assert.match(d, /expiry/i);
  // the old plan (a settle job for every backer, created together with the pledge) is marked as parked
  assert.match(d, /parked/i);
});
