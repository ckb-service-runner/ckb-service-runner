// A8 Phase 3: dashboard flow "send CKB to the runner wallet, sign a text in your wallet", and documentation.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, cleanup, startCsr, runCsr, getJson } from '../a1/lib.mjs';
import { rpc, csrDev, csrJson, waitCommitted } from '../a3b/devnet.mjs';
import { deploy, owner, deposit, attestFile } from '../a7/flow.mjs';
import { CKB, runner, fundRunnerCell } from './flow.mjs';
import { consentText, secpAddress } from './addr.mjs';

after(cleanup);

const read = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');

test('dashboard: runner wallet, text to sign, sweep settings; no payout cell, no wallet-signed transactions', () => {
  const js = read('runner/assets/app.js');
  for (const s of ['/api/wallet', '/api/registry/consent-message', '/api/registry/create', '/api/registry/remove', 'runner_address',
    'Text to sign', 'Signature', 'payout.sweep_address', 'payout.sweep_threshold_ckb', 'payout.sweep_amount_ckb']) {
    assert.ok(js.includes(s), `app.js must contain ${s}`);
  }
  for (const s of ['/api/payout', '/api/tx/submit-signed', 'prepare-create', 'prepare-withdraw', 'payout.lock_args']) {
    assert.ok(!js.includes(s), `app.js must not contain ${s} any more`);
  }
});

async function session(base) {
  const r = await fetch(`${base}/api/session`);
  const cookie = (r.headers.getSetCookie() || []).map((c) => c.split(';')[0]).find((c) => c.startsWith('csr_session='));
  return { cookie, csrf: (await r.json()).csrf_token };
}


const PW = 'correct horse battery';
async function loginSession(base) {
  const r0 = await fetch(`${base}/api/session`);
  const c0 = (r0.headers.getSetCookie() || []).map((c) => c.split(';')[0]).find((c) => c.startsWith('csr_session='));
  const s0 = { cookie: c0, csrf: (await r0.json()).csrf_token };
  const r = await fetch(`${base}/api/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', cookie: s0.cookie, 'x-csrf-token': s0.csrf },
    body: JSON.stringify({ password: PW }),
  });
  assert.equal(r.status, 200);
  const c1 = (r.headers.getSetCookie() || []).map((c) => c.split(';')[0]).find((c) => c.startsWith('csr_session='));
  return { cookie: c1 || s0.cookie, csrf: (await r.json()).csrf_token };
}
async function setPassword(args) {
  const sp = await runCsr([...args, 'set-password', '--password-stdin'], { input: `${PW}\n` });
  assert.equal(sp.code, 0, sp.stderr);
}

async function post(base, s, url, body) {
  const r = await fetch(`${base}${url}`, {
    method: 'POST', body: JSON.stringify(body),
    headers: { 'content-type': 'application/json', cookie: s.cookie, 'x-csrf-token': s.csrf },
  });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  return { status: r.status, json, text };
}

test('dashboard API: consent message, register create, register remove', async () => {
  await deploy();
  const o = await owner();
  const r = await runner('a8-api');
  await fundRunnerCell(r, 500);
  const open = await runner('a8-open');
  await startCsr([...open.args, 'run'], { healthUrl: `${open.base}/healthz` });
  const so = await session(open.base);
  for (const url of ['/api/registry/create', '/api/registry/remove']) {
    const refused = await post(open.base, so, url, {});
    assert.equal(refused.status, 403, refused.text);
    assert.equal(refused.json.error, 'password_required');
  }
  await setPassword(r.args);
  await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const s = await loginSession(r.base);

  // Since the dashboard security fixes (Oct 2026) the read APIs need the session once a password is set.
  const auth = { cookie: s.cookie };
  const rc = (await getJson(`${r.base}/api/runner-cell`, auth)).json;
  assert.equal(rc.runner_address, secpAddress(r.id.runner_lock_args, 'devnet'));
  const w = (await getJson(`${r.base}/api/wallet`, auth)).json;
  assert.equal(w.capacity_shannons, 500 * CKB);

  const stake = await deposit(10000);
  const peer = (await rpc('local_node_info')).node_id;
  const cm = await post(r.base, s, '/api/registry/consent-message', { stake });
  assert.equal(cm.status, 200, cm.text);
  const text = consentText({ network: 'devnet', stake, servicePubkey: r.id.service_pubkey, peerId: peer });
  assert.deepEqual(cm.json, { message: text, stake_owner_lock_args: o.lock_args, stake_owner_address: secpAddress(o.lock_args, 'devnet') });

  const sig = await csrDev(['sign-message', '--text', text, '--key', 'genesis']);
  assert.equal(sig.code, 0, sig.stderr);
  const att = JSON.parse(fs.readFileSync(await attestFile(r, peer, stake), 'utf8'));
  const bad = await post(r.base, s, '/api/registry/create', { stake, consent: '0x' + '00'.repeat(65), attestation: att });
  assert.equal(bad.status, 400, bad.text);
  assert.match(bad.json.error, /consent|signature/i);
  const ok = await post(r.base, s, '/api/registry/create', { stake, consent: sig.json.signature, attestation: att });
  assert.equal(ok.status, 200, ok.text);
  assert.equal(ok.json.updated, false);
  await waitCommitted(ok.json.tx_hash);
  const reg = (await getJson(`${r.base}/api/registry`, auth)).json;
  assert.equal(reg.registered, true);
  assert.equal(reg.stake.status, 'deposited');
  assert.equal(reg.stake.capacity_shannons, 10000 * CKB);

  const rm = await post(r.base, s, '/api/registry/remove', {});
  assert.equal(rm.status, 200, rm.text);
  await waitCommitted(rm.json.tx_hash);
  assert.equal((await csrJson([...r.args, 'register', 'status'])).json.registered, false);
});

test('documentation', () => {
  const t = read('docs/testing.md');
  for (const s of ['register consent-message', 'register create', 'register remove', 'wallet sweep', 'wallet send', 'sweep_address',
    'sign-message', 'reclaim', '--reuse-unchanged']) {
    assert.ok(t.includes(s), `docs/testing.md: ${s}`);
  }
  assert.match(t, /Neuron/);
  assert.ok(fs.existsSync(path.join(ROOT, 'docs', 'milestones', 'A8.md')));
  assert.match(read('CHANGELOG.md'), /A8/);
  assert.match(read('README.md'), /sign/i);
});
