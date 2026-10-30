import { describe, test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { setup, startCsr, getJson, waitFor, cleanup, blockHash } from './lib.mjs';
import { newKeyPair, makeChallenge, verify, isLowS, toHighS, answerMsg, bindingMsg, ANSWER_FIELDS, pubFromPrivHex } from './crypto.mjs';

after(cleanup);

const mon = newKeyPair();
const pingCfg = (mode, join = true) => (c) => {
  c.ranking.join = join;
  c.ranking.ping_mode = mode;
  c.monitor.pubkey = mon.pubHex;
  return c;
};
async function post(url, body) {
  const r = await fetch(url, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: typeof body === 'string' ? body : JSON.stringify(body),
  });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  return { status: r.status, json, text };
}
const is4xx = (s) => s >= 400 && s < 500;
async function startPing(name, mode = 'inbound', join = true) {
  const env = await setup(name, { cfg: pingCfg(mode, join) });
  const csr = await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  await waitFor(async () => (await getJson(`${env.base}/api/status`)).json.r2.state === 'Synced');
  return { env, csr };
}

describe('inbound: valid challenges', () => {
  let env;
  before(async () => { ({ env } = await startPing('p6-valid')); });

  test('signed answer with correct chain data', async () => {
    const c = makeChallenge(mon, { height: 900 });
    const r = await post(`${env.ping}/v1/challenge`, c);
    assert.equal(r.status, 200, r.text);
    const { answer, sig } = r.json;
    for (const f of ANSWER_FIELDS) assert.ok(f in answer, `answer.${f} missing`);
    const st = (await getJson(`${env.base}/api/status`)).json;
    const key = fs.readFileSync(path.join(env.dir, 'testnet', 'keys', 'service.key'), 'utf8');
    assert.equal(answer.nonce, c.nonce);
    assert.equal(answer.peer_id, env.mock.state.nodeId);
    assert.equal(answer.service_pubkey, st.service_pubkey);
    assert.equal(answer.service_pubkey, pubFromPrivHex(key));
    assert.equal(answer.network, 'testnet');
    assert.match(answer.csr_version, /^\d+\.\d+\.\d+/);
    assert.equal(answer.node_version, '0.210.0 (mock)');
    assert.equal(answer.r2_state, 'Synced');
    assert.equal(answer.tip_number, 1000);
    assert.equal(answer.tip_hash, blockHash(1000));
    assert.equal(answer.block_hash_at_height, blockHash(900));
    assert.equal(typeof answer.uptime_s, 'number');
    assert.equal(answer.ping_mode, 'inbound');
    assert.ok(verify(answer.service_pubkey, answerMsg(answer), sig), 'answer signature does not verify');
    assert.ok(isLowS(sig), 'answer signature must be low-S');
    assert.ok(verify(answer.service_pubkey, bindingMsg(answer.peer_id, answer.service_pubkey, answer.network), answer.binding_sig),
      'binding signature does not verify');
  });

  test('high-S monitor signatures are accepted (S is normalized)', async () => {
    const c = makeChallenge(mon, { height: 950 });
    c.monitor_sig = toHighS(c.monitor_sig);
    const r = await post(`${env.ping}/v1/challenge`, c);
    assert.equal(r.status, 200, r.text);
  });

  test('answers report the real R2 state', async () => {
    env.mock.set({ ibd: true });
    await waitFor(async () => (await getJson(`${env.base}/api/status`)).json.r2.state === 'Syncing', 6000);
    const r = await post(`${env.ping}/v1/challenge`, makeChallenge(mon, { height: 800 }));
    assert.equal(r.status, 200, r.text);
    assert.equal(r.json.answer.r2_state, 'Syncing');
    assert.ok(verify(r.json.answer.service_pubkey, answerMsg(r.json.answer), r.json.sig));
    env.mock.set({ ibd: false });
  });

  test('status shows the last successful check', async () => {
    const st = (await getJson(`${env.base}/api/status`)).json;
    assert.equal(typeof st.ranking.last_check_ok_at, 'number');
    assert.equal(st.ranking.join, true);
    assert.equal(st.ranking.ping_mode, 'inbound');
  });
});

describe('inbound: rejected challenges', () => {
  let env;
  before(async () => { ({ env } = await startPing('p6-reject')); });
  const now = () => Math.floor(Date.now() / 1000);

  test('stale and future challenges', async () => {
    assert.ok(is4xx((await post(`${env.ping}/v1/challenge`, makeChallenge(mon, { height: 900, issuedAt: now() - 120 }))).status));
    assert.ok(is4xx((await post(`${env.ping}/v1/challenge`, makeChallenge(mon, { height: 900, issuedAt: now() + 120 }))).status));
  });

  test('wrong signer and tampered challenge', async () => {
    const other = newKeyPair();
    assert.ok(is4xx((await post(`${env.ping}/v1/challenge`, makeChallenge(other, { height: 900 }))).status));
    const c = makeChallenge(mon, { height: 900 });
    c.height = 901;
    assert.ok(is4xx((await post(`${env.ping}/v1/challenge`, c)).status));
  });

  test('height above tip is answered with an empty block hash', async () => {
    const r = await post(`${env.ping}/v1/challenge`, makeChallenge(mon, { height: 5000 }));
    assert.equal(r.status, 200);
    assert.equal(r.json.answer.block_hash_at_height, '');
  });

  test('replayed nonce', async () => {
    const c = makeChallenge(mon, { height: 900 });
    assert.equal((await post(`${env.ping}/v1/challenge`, c)).status, 200);
    assert.ok(is4xx((await post(`${env.ping}/v1/challenge`, c)).status));
  });

  test('malformed body and other routes', async () => {
    assert.ok(is4xx((await post(`${env.ping}/v1/challenge`, '{"nonce":1')).status));
    assert.ok(is4xx((await fetch(`${env.ping}/api/status`)).status));
  });
});

describe('inbound: rate limit', () => {
  let env;
  before(async () => { ({ env } = await startPing('p6-rate')); });
  test('11th request within a minute gets 429', async () => {
    const statuses = [];
    for (let i = 0; i < 12; i += 1) {
      statuses.push((await post(`${env.ping}/v1/challenge`, makeChallenge(mon, { height: 900 }))).status);
    }
    assert.deepEqual(statuses.slice(0, 10), Array(10).fill(200), `statuses: ${statuses}`);
    assert.equal(statuses[10], 429, `statuses: ${statuses}`);
  });
});

describe('ping port closed when not joined', () => {
  let env;
  before(async () => { ({ env } = await startPing('p6-nojoin', 'inbound', false)); });
  test('no listener', async () => {
    await assert.rejects(fetch(`${env.ping}/v1/challenge`, { method: 'POST', body: '{}' }));
  });
});
