// A7 Phase 5: dashboard "Stake & payout", stake from the monitor, documentation.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, startCsr, getJson, cleanup } from '../a1/lib.mjs';
import { newKeyPair } from '../a1/crypto.mjs';
import { startPublicMonitor, rankedNode } from '../a1p/netmon.mjs';

after(cleanup);

const read = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');

test('dashboard: the wallet page replaces the float and payout pages', () => {
  const html = read('runner/assets/index.html');
  assert.ok(html.includes('href="#/wallet"'), 'navigation link #/wallet missing');
  assert.ok(!html.includes('#/float'), 'the #/float link must be gone');
  const js = read('runner/assets/app.js');
  assert.ok(!/\/api\/float/.test(js), 'app.js must not call /api/float any more');
  assert.ok(js.includes('stake'));
  assert.ok(!/pages\.float\b/.test(js));
});

test('the CSR monitor tab passes the monitor stake object through', async () => {
  const NODE_ID = 'QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA';
  const node = { ...rankedNode(NODE_ID), stake: { status: 'deposited', capacity_ckb: 10000, outpoint: '0x' + 'ab'.repeat(32) + ':0' } };
  const m = await startPublicMonitor({ nodes: { [NODE_ID]: node } });
  const mon = newKeyPair();
  const env = await setup('a7-monstake', {
    cfg: (c) => { c.ranking.join = true; c.monitor.url = m.url; c.monitor.pubkey = mon.pubHex; return c; },
  });
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  const j = (await getJson(`${env.base}/api/network`)).json;
  assert.deepEqual(j.node.stake, node.stake);
  const m2 = await startPublicMonitor({ nodes: { [NODE_ID]: rankedNode(NODE_ID) } });
  const env2 = await setup('a7-monstake-old', {
    cfg: (c) => { c.ranking.join = true; c.monitor.url = m2.url; c.monitor.pubkey = mon.pubHex; return c; },
  });
  await startCsr([...env2.args, 'run'], { healthUrl: `${env2.base}/healthz` });
  const j2 = (await getJson(`${env2.base}/api/network`)).json;
  assert.equal(j2.node.rank, 7, 'an older monitor without stake still works');
  assert.equal(j2.node.stake, null);
});

test('documentation', () => {
  assert.equal(fs.existsSync(path.join(ROOT, 'spec', 'float-lock.md')), false, 'spec/float-lock.md must be replaced');
  const dual = read('spec/dual-lock.md');
  for (const s of ['owner_lock_hash', 'runner_lock_hash']) assert.ok(dual.includes(s), `dual-lock.md must cover ${s}`);
  const job = read('spec/job-cell-standard.md');
  assert.ok(job.includes('payout_lock_hash') && !job.includes('float_lock_hash'));
  assert.match(job, /anyone-can-pay/i);
  const api = read('spec/monitor-public-api.md');
  for (const s of ['"stake"', 'stake_invalid', 'stake_in_use', 'deposited', 'withdrawn']) assert.ok(api.includes(s), `monitor-public-api.md: ${s}`);
  const dec = read('docs/decisions.md');
  assert.match(dec, /D-A7-1/);
  assert.ok(fs.existsSync(path.join(ROOT, 'docs', 'milestones', 'A7.md')));
  assert.match(read('CHANGELOG.md'), /A7/);
  const testing = read('docs/testing.md');
  for (const s of ['dao-deposit', 'register create', '--stake']) assert.ok(testing.includes(s), `testing.md: ${s}`);
  assert.ok(!testing.includes('float prepare-create'));
  const ex = read('config/csr.example.toml');
  assert.match(ex, /^\[payout\]$/m);
  for (const f of ['README.md', 'docs/install.md', 'docs/architecture.md']) {
    assert.ok(!/csr float /.test(read(f)), `${f} must not mention csr float commands`);
  }
});
