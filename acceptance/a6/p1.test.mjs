// A6 Phase 1: /api/network passes the monitor's map locations through.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { setup, startCsr, getJson, cleanup } from '../a1/lib.mjs';
import { newKeyPair } from '../a1/crypto.mjs';
import { startMapMonitor, NETWORK } from './netmon.mjs';

after(cleanup);

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

const LOCS = {
  locations: [
    { lat: 53, lon: 13, count: 2 }, { lat: 41, lon: -75, count: 1 },
    { lat: 95, lon: 0, count: 1 }, { lat: 10, lon: -181, count: 1 }, { lat: 1, lon: 1, count: 0 },
    { lat: '5', lon: 1, count: 1 }, { lat: 1, lon: 1, count: 1.5 }, { lat: -33, lon: 151, count: 1 },
  ],
  located: 4, unlocated: 3, grid_deg: 2, updated_at: 1792000000,
};

test('locations are passed through, invalid entries dropped, cached', async () => {
  const m = await startMapMonitor({ locations: LOCS });
  const env = await start('a6-locs', m);
  const r = await getJson(`${env.base}/api/network`);
  assert.equal(r.status, 200, r.text);
  const nw = r.json.network;
  assert.equal(r.json.stale, false);
  assert.equal(r.json.error, null);
  assert.equal(nw.nodes_total, NETWORK.nodes_total);
  assert.deepEqual(nw.countries, NETWORK.countries);
  assert.equal(nw.locations_available, true);
  assert.deepEqual(nw.locations, [{ lat: 53, lon: 13, count: 2 }, { lat: 41, lon: -75, count: 1 }, { lat: -33, lon: 151, count: 1 }]);
  assert.equal(nw.unlocated, 3);
  await getJson(`${env.base}/api/network`);
  await getJson(`${env.base}/api/network`);
  assert.equal(m.hits['/v1/locations'], 1);
  assert.equal(m.hits['/v1/network'], 1);
});

test('monitor without /v1/locations: empty map, no error', async () => {
  const m = await startMapMonitor();
  const env = await start('a6-nolocs', m);
  const j = (await getJson(`${env.base}/api/network`)).json;
  assert.equal(j.error, null);
  assert.equal(j.stale, false);
  assert.equal(j.network.nodes_total, NETWORK.nodes_total);
  assert.deepEqual(j.network.locations, []);
  assert.equal(j.network.unlocated, 0);
  assert.equal(j.network.locations_available, false);
});

test('bad /v1/locations reply: empty map, no error', async () => {
  const m = await startMapMonitor({ locations: 'bad' });
  const env = await start('a6-badlocs', m);
  const j = (await getJson(`${env.base}/api/network`)).json;
  assert.equal(j.error, null);
  assert.deepEqual(j.network.locations, []);
  assert.equal(j.network.locations_available, false);
});

test('at most 2000 locations', async () => {
  const many = [];
  for (let i = 0; i < 2100; i++) many.push({ lat: -89 + 2 * Math.floor(i / 180), lon: -179 + 2 * (i % 180), count: 1 });
  const m = await startMapMonitor({ locations: { locations: many, unlocated: 0 } });
  const env = await start('a6-many', m);
  const j = (await getJson(`${env.base}/api/network`)).json;
  assert.equal(j.network.locations.length, 2000);
  assert.deepEqual(j.network.locations[0], { lat: -89, lon: -179, count: 1 });
});

test('not joined: new wording, no monitor contact', async () => {
  const m = await startMapMonitor({ locations: LOCS });
  const env = await start('a6-nojoin', m, false);
  const j = (await getJson(`${env.base}/api/network`)).json;
  assert.equal(j.join, false);
  assert.equal(j.message, 'Join the CSR monitor to see stats');
  assert.deepEqual(m.hits, {});
});
