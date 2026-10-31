import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, startCsr, getJson, waitFor, sleep, cleanup } from './lib.mjs';
import { newKeyPair, makeChallenge, verify, answerMsg, bindingMsg } from './crypto.mjs';
import { startMockMonitor } from './monitor.mjs';

after(cleanup);

const mon = newKeyPair();
async function start(name, mode, monitor, join = true) {
  const env = await setup(name, {
    cfg: (c) => {
      c.ranking.join = join;
      c.ranking.ping_mode = mode;
      c.monitor.url = monitor.url;
      c.monitor.pubkey = mon.pubHex;
      return c;
    },
  });
  const csr = await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  return { env, csr };
}

test('outbound check-in answers the monitor', async () => {
  const m = await startMockMonitor(mon);
  const { env } = await start('p7-out', 'outbound', m);
  const a = await waitFor(() => m.answers.find((x) => x && x.answer && x.answer.r2_state === 'Synced'), 15000);
  const st = (await getJson(`${env.base}/api/status`)).json;
  const ci = m.checkins[0];
  assert.equal(ci.peer_id, env.mock.state.nodeId);
  assert.equal(ci.service_pubkey, st.service_pubkey);
  assert.equal(ci.network, 'testnet');
  assert.match(ci.csr_version, /^\d+\.\d+\.\d+/);
  assert.ok(verify(ci.service_pubkey, bindingMsg(ci.peer_id, ci.service_pubkey, ci.network), ci.binding_sig), 'binding signature');
  assert.ok(m.issued.has(a.answer.nonce), 'answer must echo an issued nonce');
  assert.equal(a.answer.ping_mode, 'outbound');
  assert.ok(verify(a.answer.service_pubkey, answerMsg(a.answer), a.sig), 'answer signature');
  await assert.rejects(fetch(`${env.ping}/v1/challenge`, { method: 'POST', body: '{}' }), 'outbound mode must not open the ping port');
  await waitFor(async () => typeof (await getJson(`${env.base}/api/status`)).json.ranking.last_check_ok_at === 'number', 6000);
});

test('both modes at once', async () => {
  const m = await startMockMonitor(mon);
  const { env } = await start('p7-both', 'both', m);
  await waitFor(() => m.answers.length > 0, 15000);
  assert.equal(m.answers[0].answer.ping_mode, 'both');
  await waitFor(async () => (await getJson(`${env.base}/api/status`)).json.r2.state === 'Synced');
  const r = await fetch(`${env.ping}/v1/challenge`, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify(makeChallenge(mon, { height: 900 })),
  });
  assert.equal(r.status, 200);
});

test('invalid monitor challenges are not answered', async () => {
  const m = await startMockMonitor(mon, { badSig: true });
  await start('p7-bad', 'outbound', m);
  await waitFor(() => m.checkins.length >= 2, 15000);
  assert.equal(m.answers.length, 0, 'answered a challenge with a bad monitor signature');
});

test('no check-ins when not joined', async () => {
  const m = await startMockMonitor(mon);
  await start('p7-nojoin', 'outbound', m, false);
  await sleep(6000);
  assert.equal(m.checkins.length, 0);
});

test('documentation', () => {
  const protocol = fs.readFileSync(path.join(ROOT, 'spec', 'ping-protocol.md'), 'utf8');
  for (const s of ['CSR-PING-CHALLENGE-v1', 'CSR-PING-ANSWER-v1', 'CSR-BINDING-v1', '/v1/challenge', '/v1/checkin', '/v1/answer']) {
    assert.ok(protocol.includes(s), `spec/ping-protocol.md must describe ${s}`);
  }
  const readme = fs.readFileSync(path.join(ROOT, 'README.md'), 'utf8');
  assert.match(readme, /csr run/);
  assert.match(readme, /csr detect/);
  assert.doesNotMatch(readme, /mempool/i, 'README still mentions the mempool fork');
  assert.match(fs.readFileSync(path.join(ROOT, 'CHANGELOG.md'), 'utf8'), /A1/);
  assert.ok(fs.existsSync(path.join(ROOT, 'runner', 'tests', 'mock_monitor')), 'runner/tests/mock_monitor missing');
});
