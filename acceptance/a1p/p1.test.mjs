import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { setup, startCsr, getJson, cleanup } from '../a1/lib.mjs';
import { newKeyPair } from '../a1/crypto.mjs';
import { startPublicMonitor, rankedNode, NETWORK } from './netmon.mjs';

after(cleanup);

const NODE_ID = 'QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA';
const mon = newKeyPair();
async function start(name, monitor, join = true) {
  const env = await setup(name, {
    cfg: (c) => {
      c.ranking.join = join;
      c.ranking.ping_mode = 'inbound';
      c.monitor.url = monitor.url;
      c.monitor.pubkey = mon.pubHex;
      return c;
    },
  });
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  return env;
}

test('not joined: no monitor contact', async () => {
  const m = await startPublicMonitor({ nodes: { [NODE_ID]: rankedNode(NODE_ID) } });
  const env = await start('a1p-nojoin', m, false);
  const r = await getJson(`${env.base}/api/network`);
  assert.equal(r.status, 200);
  assert.equal(r.json.join, false);
  assert.match(r.json.message, /join/i);
  assert.deepEqual(m.hits, {}, 'the monitor must not be contacted when not joined');
});

test('joined and ranked; responses are cached', async () => {
  const m = await startPublicMonitor({ nodes: { [NODE_ID]: rankedNode(NODE_ID) } });
  const env = await start('a1p-ranked', m);
  const r = await getJson(`${env.base}/api/network`);
  assert.equal(r.status, 200, r.text);
  const j = r.json;
  assert.equal(j.join, true);
  assert.equal(j.stale, false);
  assert.equal(j.network.nodes_total, NETWORK.nodes_total);
  assert.equal(j.network.nodes_reachable, NETWORK.nodes_reachable);
  assert.deepEqual(j.network.countries, NETWORK.countries);
  assert.equal(j.node.rank, 7);
  assert.equal(j.node.of, 42);
  assert.equal(j.node.availability.d30, 0.97);
  assert.equal(j.node.check_type, 'p2p_verified');
  assert.equal(j.node.attestation.status, 'valid');
  assert.equal(j.ping.mode, 'inbound');
  const before = { ...m.hits };
  await getJson(`${env.base}/api/network`);
  await getJson(`${env.base}/api/network`);
  assert.deepEqual(m.hits, before, 'cached responses must not trigger new monitor requests');
  assert.equal(m.hits['/v1/network'], 1);
});

test('joined but not ranked', async () => {
  const m = await startPublicMonitor();
  const env = await start('a1p-unranked', m);
  const j = (await getJson(`${env.base}/api/network`)).json;
  assert.equal(j.node, null);
  assert.equal(j.network.nodes_total, NETWORK.nodes_total);
});

test('monitor down', async () => {
  const m = await startPublicMonitor({ down: true });
  const env = await start('a1p-down', m);
  const r = await getJson(`${env.base}/api/network`);
  assert.equal(r.status, 200);
  assert.equal(r.json.network, null);
  assert.ok(typeof r.json.error === 'string' && r.json.error.length > 0);
});
