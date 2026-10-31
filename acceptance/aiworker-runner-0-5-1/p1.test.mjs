// 0.5.1: mainnet notices and locked registration, per-network session cookie, Phase 2 gate on the dashboard.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { setup, startCsr, runCsr, getJson, cleanup } from '../a1/lib.mjs';

after(cleanup);
const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const PW = 'correct horse battery';

function cookies(res) {
  return (res.headers.getSetCookie() || []).map((c) => c.split(';')[0]);
}
async function login(base) {
  const r0 = await fetch(`${base}/api/session`);
  const c0 = cookies(r0)[0];
  const i0 = await r0.json();
  const r = await fetch(`${base}/api/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', cookie: c0, 'x-csrf-token': i0.csrf_token },
    body: JSON.stringify({ password: PW }),
  });
  assert.equal(r.status, 200);
  const j = await r.json();
  const c = cookies(r)[0] || c0;
  return { first: c0, cookie: c, csrf: j.csrf_token || i0.csrf_token };
}
async function post(base, p, body, s) {
  const r = await fetch(`${base}${p}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', cookie: s.cookie, 'x-csrf-token': s.csrf },
    body: JSON.stringify(body),
  });
  return { status: r.status, json: await r.json().catch(() => null) };
}
async function runner(name, network) {
  const env = await setup(name, {
    node: network === 'mainnet' ? { chain: 'ckb' } : {},
    cfg: (c) => { c.network = network; return c; },
  });
  const sp = await runCsr([...env.args, 'set-password', '--password-stdin'], { input: `${PW}\n` });
  assert.equal(sp.code, 0, sp.stderr);
  const extra = network === 'mainnet' ? ['--confirm-mainnet'] : [];
  await startCsr([...env.args, 'run', ...extra], { healthUrl: `${env.base}/healthz` });
  return env;
}

test('mainnet: registration is locked with a notice, the wallet stays usable', async () => {
  const env = await runner('v051-main', 'mainnet');
  const s = await login(env.base);
  assert.match(s.first, /^csr_session_mainnet=/, 'the mainnet dashboard uses its own cookie');
  const st = await getJson(`${env.base}/api/status`, { cookie: s.cookie });
  assert.equal(st.status, 200, st.text);
  assert.match(st.json.registration_locked, /CSR 1\.x/);
  assert.match(st.json.registration_locked, /20 ranked runners/);
  const reg = await getJson(`${env.base}/api/registry`, { cookie: s.cookie });
  assert.equal(reg.status, 200, reg.text);
  assert.equal(reg.json.locked, true);
  assert.equal(reg.json.registered, false);
  assert.match(reg.json.message, /Nothing needs to be funded/);
  for (const [p, body] of [['/api/registry/consent-message', { stake: '0x' + '11'.repeat(32) + ':0' }],
    ['/api/registry/create', { stake: '0x' + '11'.repeat(32) + ':0', consent: '0x00' }],
    ['/api/registry/refresh', {}], ['/api/registry/remove', {}]]) {
    const r = await post(env.base, p, body, s);
    assert.equal(r.status, 409, p);
    assert.equal(r.json.error, 'mainnet_locked', p);
    assert.match(r.json.detail, /CSR 1\.x/, p);
  }
  const cli = await runCsr([...env.args, 'register', 'consent-message', '--stake', '0x' + '11'.repeat(32) + ':0']);
  assert.notEqual(cli.code, 0);
  assert.match(cli.stderr, /CSR 1\.x/);
});

test('testnet: nothing locked, the classic cookie', async () => {
  const env = await runner('v051-test', 'testnet');
  const s = await login(env.base);
  assert.match(s.first, /^csr_session=/);
  const st = await getJson(`${env.base}/api/status`, { cookie: s.cookie });
  assert.equal(st.json.registration_locked, null);
  const r = await post(env.base, '/api/registry/consent-message', { stake: '0x' + '11'.repeat(32) + ':0' }, s);
  assert.notEqual(r.json && r.json.error, 'mainnet_locked');
});

test('the dashboard shows the notice and the gate', () => {
  const js = read('runner', 'assets', 'app.js');
  for (const s of ['function lockedNotice', 'function gateMetrics', 'lockedTop(root)', "error === 'mainnet_locked'", 'r[1].locked', 'j.gate']) {
    assert.ok(js.includes(s), s);
  }
  assert.match(read('runner', 'assets', 'app.css'), /\.locked-notice\{/);
  assert.match(read('runner', 'src', 'monitor', 'mod.rs'), /v1\/\{\}\/summary/);
  assert.match(read('runner', 'Cargo.toml'), /^version = "0\.5\.1"$/m);
  assert.match(read('CHANGELOG.md'), /^## 0\.5\.1$/m);
});
