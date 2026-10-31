// csr-mainnet-dashboard, phase 1: no network toggle, confirm-mainnet, link to the other instance (D31, D32).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const BIN = path.join(ROOT, 'target', 'debug', 'csr');
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

function tmp() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'csr-p1-'));
}

function cfg(dir, name, body) {
  const f = path.join(dir, name);
  fs.writeFileSync(f, body);
  return f;
}

function csr(dir, conf, ...args) {
  return spawnSync(BIN, ['--data-root', dir, '--config', conf, ...args], { encoding: 'utf8', timeout: 60000 });
}

test('the binary is built', () => {
  assert.ok(fs.existsSync(BIN), 'run cargo build -p csr first');
});

test('confirm-mainnet writes the marker on a mainnet config', () => {
  const d = tmp();
  const c = cfg(d, 'csr-mainnet.toml', 'network = "mainnet"\n');
  const r = csr(d, c, 'confirm-mainnet');
  assert.equal(r.status, 0, r.stderr);
  assert.ok(fs.existsSync(path.join(d, 'mainnet', '.mainnet-confirmed')));
  const again = csr(d, c, 'confirm-mainnet');           // idempotent
  assert.equal(again.status, 0, again.stderr);
});

test('confirm-mainnet refuses a testnet config and writes nothing', () => {
  const d = tmp();
  const c = cfg(d, 'csr-testnet.toml', 'network = "testnet"\n');
  const r = csr(d, c, 'confirm-mainnet');
  assert.notEqual(r.status, 0);
  assert.match(r.stdout + r.stderr, /mainnet/);
  assert.ok(!fs.existsSync(path.join(d, 'mainnet', '.mainnet-confirmed')));
  assert.ok(!fs.existsSync(path.join(d, 'testnet', '.mainnet-confirmed')));
});

test('dashboard.peer_url is validated', () => {
  const d = tmp();
  const ok = (v) => csr(d, cfg(d, 'c.toml', `network = "testnet"\n[dashboard]\npeer_url = "${v}"\n`), 'config', 'validate');
  assert.equal(ok('').status, 0);
  assert.equal(ok('http://127.0.0.1:8140').status, 0);
  assert.equal(ok('https://node.example:8140/').status, 0);
  for (const bad of ['ftp://127.0.0.1:8140', '127.0.0.1:8140', 'http://', 'http://' + 'a'.repeat(200) + ':1']) {
    assert.notEqual(ok(bad).status, 0, bad);
  }
});

test('the example config has the new key', () => {
  const t = read('config', 'csr.example.toml');
  const dash = t.split('[dashboard]')[1].split('\n[')[0];
  assert.match(dash, /^peer_url = ""/m);
});

test('the settings page cannot change the network', () => {
  const js = read('runner', 'assets', 'app.js');
  assert.doesNotMatch(js, /\['network',\s*'Network',\s*'select:/);
  const rs = read('runner', 'src', 'web', 'settings.rs');
  assert.match(rs, /network follows the node/);
});

test('status and header link to the other instance', () => {
  assert.match(read('runner', 'src', 'app.rs'), /"peer_dashboard"/);
  assert.match(read('runner', 'assets', 'index.html'), /id="peer-dashboard"/);
  const js = read('runner', 'assets', 'app.js');
  assert.match(js, /peer_dashboard/);
  assert.match(js, /peer-dashboard/);
});

// ---- live API (mock node and helpers of acceptance/a1) ----
import { after } from 'node:test';
import { setup, startCsr, runCsr, getJson, cleanup } from '../a1/lib.mjs';

after(cleanup);

const PW = 'correct horse battery';
function sessionCookie(res) {
  return (res.headers.getSetCookie() || []).map((c) => c.split(';')[0]).find((c) => c.startsWith('csr_session='));
}
async function login(base) {
  const r0 = await fetch(`${base}/api/session`);
  const c0 = sessionCookie(r0);
  const i0 = await r0.json();
  const r = await fetch(`${base}/api/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', cookie: c0, 'x-csrf-token': i0.csrf_token },
    body: JSON.stringify({ password: PW }),
  });
  assert.equal(r.status, 200);
  const j = await r.json();
  return { cookie: sessionCookie(r) || c0, csrf: j.csrf_token || i0.csrf_token };
}
async function put(base, body, s) {
  const r = await fetch(`${base}/api/settings`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json', cookie: s.cookie, 'x-csrf-token': s.csrf },
    body: JSON.stringify(body),
  });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  return { status: r.status, json, text };
}

test('live: status carries peer_dashboard, settings refuse a network change', async () => {
  const env = await setup('mdash-p1', { cfg: (c) => { c.dashboard.peer_url = 'http://127.0.0.1:8140'; return c; } });
  const sp = await runCsr([...env.args, 'set-password', '--password-stdin'], { input: `${PW}\n` });
  assert.equal(sp.code, 0, sp.stderr);
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  const s = await login(env.base);
  const st = await getJson(`${env.base}/api/status`, { cookie: s.cookie });
  assert.equal(st.status, 200, st.text);
  assert.equal(st.json.peer_dashboard, 'http://127.0.0.1:8140');
  assert.equal(st.json.network, 'testnet');
  const before = fs.readFileSync(env.configPath, 'utf8');
  const r = await put(env.base, { network: 'mainnet' }, s);
  assert.equal(r.status, 400, r.text);
  assert.match(r.json.error, /network follows the node/);
  assert.equal(fs.readFileSync(env.configPath, 'utf8'), before);
  const ok = await put(env.base, { logging: { level: 'warn' } }, s);   // other settings still work
  assert.equal(ok.status, 200, ok.text);
});

test('live: no peer_url gives peer_dashboard null', async () => {
  const env = await setup('mdash-p1b');
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  const st = await getJson(`${env.base}/api/status`);
  assert.equal(st.json.peer_dashboard, null);
});
