// fix-runner-unranked-node: the CSR monitor tab accepts an unranked /v1/nodes reply (F-XC-002).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, startCsr, getJson, cleanup } from '../a1/lib.mjs';
import { newKeyPair } from '../a1/crypto.mjs';
import { startPublicMonitor, rankedNode, NETWORK } from '../a1p/netmon.mjs';

after(cleanup);

const NODE_ID = 'QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA';
const STAKE = { status: 'deposited', capacity_ckb: 10000, outpoint: '0x' + 'ab'.repeat(32) + ':0' };

async function networkJson(name, node) {
  const m = await startPublicMonitor({ nodes: { [NODE_ID]: node } });
  const mon = newKeyPair();
  const env = await setup(name, {
    cfg: (c) => { c.ranking.join = true; c.monitor.url = m.url; c.monitor.pubkey = mon.pubHex; return c; },
  });
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  return (await getJson(`${env.base}/api/network`)).json;
}

test('a provisional node shows its stake and attestation', async () => {
  const node = {
    peer_id: NODE_ID, rank: null, of: 42, provisional: true,
    availability: { d7: 0.5, d30: 0.5, d365: 0.5 }, check_type: 'challenge_only',
    attestation: { status: 'valid', expiry: 1800000000 }, stake: STAKE, last_check_at: 1792000100,
  };
  const j = await networkJson('fix-unranked-prov', node);
  assert.equal(j.error, null, `error: ${j.error}`);
  assert.equal(j.stale, false);
  assert.equal(j.network.nodes_total, NETWORK.nodes_total);
  assert.equal(j.node.rank, null);
  assert.equal(j.node.provisional, true);
  assert.equal(j.node.of, 42);
  assert.deepEqual(j.node.stake, STAKE);
  assert.deepEqual(j.node.attestation, node.attestation);
});

test('a node known only from its registry cell (null availability)', async () => {
  const node = {
    peer_id: NODE_ID, rank: null, of: 42, provisional: false,
    availability: { d7: null, d30: null, d365: null }, check_type: 'none',
    attestation: { status: 'none', expiry: null },
    stake: { status: 'withdrawn', capacity_ckb: null, outpoint: STAKE.outpoint }, last_check_at: 0,
  };
  const j = await networkJson('fix-unranked-reg', node);
  assert.equal(j.error, null, `error: ${j.error}`);
  assert.equal(j.network.nodes_total, NETWORK.nodes_total);
  assert.equal(j.node.rank, null);
  assert.equal(j.node.provisional, false);
  assert.deepEqual(j.node.availability, { d7: null, d30: null, d365: null });
  assert.equal(j.node.check_type, 'none');
  assert.equal(j.node.stake.status, 'withdrawn');
});

test('a ranked node from an older monitor still works and is not provisional', async () => {
  const j = await networkJson('fix-unranked-old', rankedNode(NODE_ID));
  assert.equal(j.error, null, `error: ${j.error}`);
  assert.equal(j.node.rank, 7);
  assert.equal(j.node.of, 42);
  assert.equal(j.node.provisional, false);
  assert.equal(j.node.availability.d30, 0.97);
});

test('a malformed node reply does not hide the network data', async () => {
  const j = await networkJson('fix-unranked-bad', { ...rankedNode(NODE_ID), rank: 'seven' });
  assert.ok(j.network, 'network data must still be shown');
  assert.equal(j.network.nodes_total, NETWORK.nodes_total);
  assert.equal(j.node, null);
});

test('the public API spec describes unranked nodes', () => {
  const t = fs.readFileSync(path.join(ROOT, 'spec', 'monitor-public-api.md'), 'utf8');
  assert.ok(t.includes('"provisional"'), 'spec/monitor-public-api.md must document "provisional"');
  assert.ok(t.includes('unknown_node'), 'spec/monitor-public-api.md must document 404 unknown_node');
  assert.ok(!t.includes('404 when the node is not ranked'), 'the old 404 rule must be gone');
});
