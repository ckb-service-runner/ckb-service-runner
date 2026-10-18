import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { setup, runCsr, cleanup } from './lib.mjs';

after(cleanup);

async function check(env) {
  const r = await runCsr([...env.args, 'check', '--json'], { timeoutMs: 30000 });
  let j = null;
  try { j = JSON.parse(r.stdout); } catch { /* asserted below */ }
  assert.ok(j, `check --json must print one JSON object on stdout; got:\n${r.stdout}\n${r.stderr}`);
  return { ...r, j };
}

test('synced node passes R2', async () => {
  const env = await setup('p2-synced');
  const r = await check(env);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(r.j.network, 'testnet');
  assert.equal(r.j.config_ok, true);
  assert.equal(r.j.node.reachable, true);
  assert.equal(r.j.node.chain, 'ckb_testnet');
  assert.equal(r.j.node.node_type, 'ckb');
  assert.equal(r.j.node.node_version, '0.210.0 (mock)');
  assert.equal(r.j.r2.state, 'Synced');
  assert.deepEqual(r.j.r2.reasons, []);
  assert.equal(r.j.r2.tip, 1000);
  assert.equal(r.j.r2.best_known, 1000);
  assert.equal(r.j.r2.tip_lag, 0);
  assert.equal(r.j.r2.indexer_tip, 1000);
  assert.equal(r.j.r2.indexer_lag, 0);
  assert.equal(r.j.r2.outbound_peers, 1);
  assert.equal(r.j.r2.ibd, false);
});

const cases = [
  ['initial block download', { ibd: true }, 'Syncing', null],
  ['tip lag 5 is fine', { bestKnown: 1005 }, 'Synced', null],
  ['tip lag 6', { bestKnown: 1006 }, 'Syncing', null],
  ['node ahead of best known', { bestKnown: 990 }, 'Synced', null],
  ['indexer lag 5 is fine', { indexerTip: 995 }, 'Synced', null],
  ['indexer lag 6', { indexerTip: 994 }, 'Degraded', /indexer/i],
  ['no outbound peers', { peers: [{ is_outbound: false }] }, 'Degraded', /outbound peers/i],
  ['indexer module disabled', { indexer: false }, 'Degraded', /indexer/i],
  ['wrong chain', { chain: 'ckb' }, 'Degraded', /chain/i],
  ['light client', { light: true }, 'Degraded', /unsupported/i],
  ['IBD wins over missing peers', { ibd: true, peers: [] }, 'Syncing', null],
];
for (const [name, patch, state, re] of cases) {
  test(`R2: ${name} -> ${state}`, async () => {
    const env = await setup('p2-case', { node: patch });
    const r = await check(env);
    assert.equal(r.j.r2.state, state, JSON.stringify(r.j));
    if (state === 'Synced') {
      assert.equal(r.code, 0);
      assert.deepEqual(r.j.r2.reasons, []);
    } else {
      assert.equal(r.code, 2);
      assert.ok(r.j.r2.reasons.length > 0, 'reasons must explain the state');
    }
    if (re) assert.match(r.j.r2.reasons.join(' | '), re);
  });
}

test('R2: unreachable node', async () => {
  const env = await setup('p2-unreach');
  await env.mock.close();
  const started = Date.now();
  const r = await check(env);
  assert.ok(Date.now() - started < 15000, 'check must give up quickly');
  assert.equal(r.code, 2);
  assert.equal(r.j.node.reachable, false);
  assert.equal(r.j.r2.state, 'Unreachable');
});

test('detect: full node', async () => {
  const env = await setup('p2-detect');
  const r = await runCsr([...env.args, 'detect', '--json']);
  assert.equal(r.code, 0, r.stderr);
  const j = JSON.parse(r.stdout);
  assert.equal(j.reachable, true);
  assert.equal(j.node_type, 'ckb');
  assert.equal(j.chain, 'ckb_testnet');
  assert.equal(j.indexer_enabled, true);
});

test('detect: indexer disabled gives a hint', async () => {
  const env = await setup('p2-detect-idx', { node: { indexer: false } });
  const r = await runCsr([...env.args, 'detect', '--json']);
  assert.equal(r.code, 0, r.stderr);
  const j = JSON.parse(r.stdout);
  assert.equal(j.indexer_enabled, false);
  assert.match(j.hint, /Indexer/);
});

test('detect: light client is unsupported', async () => {
  const env = await setup('p2-detect-light', { node: { light: true } });
  const r = await runCsr([...env.args, 'detect', '--json']);
  assert.equal(r.code, 2);
  assert.equal(JSON.parse(r.stdout).node_type, 'unsupported');
});

test('detect: unreachable', async () => {
  const env = await setup('p2-detect-down');
  await env.mock.close();
  const r = await runCsr([...env.args, 'detect', '--json']);
  assert.equal(r.code, 2);
  assert.equal(JSON.parse(r.stdout).reachable, false);
});
