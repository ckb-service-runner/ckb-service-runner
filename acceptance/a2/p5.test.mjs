import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, runCsr, startCsr, getJson, cleanup } from '../a1/lib.mjs';
import { runService, manifest } from './svc.mjs';

after(cleanup);

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
async function action(base, s, hash, verb) {
  const headers = {};
  if (s) { headers.cookie = s.cookie; headers['x-csrf-token'] = s.csrf; }
  const r = await fetch(`${base}/api/services/${hash}/${verb}`, { method: 'POST', headers });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  return { status: r.status, json, text };
}

test('services registry: approve, disable, enable, revoke', async () => {
  const env = await setup('a2-reg');
  const kv = await runService(env, 'kvtest');
  const added = [];
  for (const name of ['hello-rust', 'kvtest']) {
    const r = await runCsr([...env.args, 'service', 'add-local', '--manifest', manifest(name)]);
    assert.equal(r.code, 0, r.stderr);
    added.push(JSON.parse(r.stdout));
  }
  const hello = added[0].code_hash;
  assert.match(hello, /^0x[0-9a-f]{64}$/);
  assert.equal(added[1].code_hash, kv.code_hash);

  const cli = await runCsr([...env.args, 'service', 'list', '--json']);
  assert.equal(cli.code, 0, cli.stderr);
  assert.equal(JSON.parse(cli.stdout).length, 2);

  await setPassword(env.args);
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  // Integration (Oct 2026): with a password set, every /api/* read needs the login session (dashboard-security).
  const s = await loginSession(env.base);
  const list = (await getJson(`${env.base}/api/services`, { cookie: s.cookie })).json;
  assert.equal(list.length, 2);
  for (const s of list) assert.equal(s.status, 'not_approved');
  assert.ok(list.find((s) => s.code_hash === kv.code_hash).storage_bytes > 0);

  assert.ok([401, 403].includes((await action(env.base, null, hello, 'approve')).status));
  let r = await action(env.base, s, hello, 'approve');
  assert.equal(r.status, 200, r.text);
  assert.equal(r.json.status, 'approved_enabled');
  assert.ok(fs.readFileSync(env.configPath, 'utf8').includes(hello.slice(2)), 'approved hash must be in allowed_services');
  r = await action(env.base, s, hello, 'disable');
  assert.equal(r.json.status, 'approved_disabled');
  r = await action(env.base, s, hello, 'enable');
  assert.equal(r.json.status, 'approved_enabled');
  r = await action(env.base, s, hello, 'revoke');
  assert.equal(r.json.status, 'not_approved');
  assert.ok(!fs.readFileSync(env.configPath, 'utf8').includes(hello.slice(2)), 'revoked hash must leave allowed_services');
  assert.equal((await action(env.base, s, '0x' + '00'.repeat(32), 'approve')).status, 404);
});

test('service actions without a password are refused with password_required', async () => {
  const env = await setup('a2-open');
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  const s = await session(env.base);
  const r = await action(env.base, s, '0x' + '00'.repeat(32), 'approve');
  assert.equal(r.status, 403, r.text);
  assert.equal(r.json.error, 'password_required');
});

test('services page is linked; milestone documented', () => {
  const html = fs.readFileSync(path.join(ROOT, 'runner', 'assets', 'index.html'), 'utf8');
  assert.ok(html.includes('href="#/services"'));
  assert.ok(fs.existsSync(path.join(ROOT, 'docs', 'milestones', 'A2.md')));
  assert.ok(fs.existsSync(path.join(ROOT, 'spec', 'host-interface.md')));
});
