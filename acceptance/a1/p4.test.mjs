import { describe, test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { setup, startCsr, runCsr, getJson, cleanup, freePort } from './lib.mjs';

after(cleanup);

function sessionCookie(res) {
  return (res.headers.getSetCookie() || []).map((c) => c.split(';')[0]).find((c) => c.startsWith('csr_session='));
}
async function openSession(base) {
  const r = await fetch(`${base}/api/session`);
  assert.equal(r.status, 200);
  const cookie = sessionCookie(r);
  assert.ok(cookie, 'GET /api/session must set the csr_session cookie');
  const info = await r.json();
  assert.ok(typeof info.csrf_token === 'string' && info.csrf_token.length >= 16, 'csrf_token missing');
  return { cookie, csrf: info.csrf_token, info };
}
async function putSettings(base, body, { cookie, csrf } = {}) {
  const headers = { 'content-type': 'application/json' };
  if (cookie) headers.cookie = cookie;
  if (csrf) headers['x-csrf-token'] = csrf;
  const r = await fetch(`${base}/api/settings`, { method: 'PUT', headers, body: JSON.stringify(body) });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  return { status: r.status, json, text };
}
async function firstEvent(url, name, headers = {}, timeoutMs = 8000) {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), timeoutMs);
  try {
    const r = await fetch(url, { headers, signal: ac.signal });
    assert.equal(r.status, 200);
    assert.match(r.headers.get('content-type') || '', /text\/event-stream/);
    const dec = new TextDecoder();
    let buf = '';
    for await (const chunk of r.body) {
      buf += dec.decode(chunk, { stream: true }).replace(/\r\n/g, '\n');
      const blocks = buf.split('\n\n');
      buf = blocks.pop();
      for (const b of blocks) {
        const lines = b.split('\n');
        const ev = lines.find((l) => l.startsWith('event:'));
        const data = lines.filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim()).join('\n');
        if (ev && ev.slice(6).trim() === name) return JSON.parse(data);
      }
    }
    throw new Error('event stream ended');
  } finally {
    clearTimeout(timer);
    ac.abort();
  }
}

describe('no password, loopback', () => {
  let env;
  before(async () => {
    env = await setup('p4-open');
    await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  });

  test('reads need no session', async () => {
    assert.equal((await fetch(`${env.base}/api/status`)).status, 200);
  });

  test('writes need cookie and matching CSRF token', async () => {
    const s = await openSession(env.base);
    assert.equal(s.info.password_required, false);
    assert.equal((await putSettings(env.base, { logging: { level: 'warn' } })).status, 403);
    assert.equal((await putSettings(env.base, { logging: { level: 'warn' } }, { cookie: s.cookie })).status, 403);
    assert.equal((await putSettings(env.base, { logging: { level: 'warn' } }, { cookie: s.cookie, csrf: 'x'.repeat(32) })).status, 403);
    assert.equal((await putSettings(env.base, { logging: { level: 'warn' } }, { csrf: s.csrf })).status, 403);
  });

  test('settings never expose the password hash', async () => {
    const r = await getJson(`${env.base}/api/settings`);
    assert.equal(r.status, 200);
    for (const k of ['network', 'node', 'dashboard', 'ranking', 'monitor', 'runtime', 'limits', 'storage', 'logging']) {
      assert.ok(k in r.json, `settings.${k} missing`);
    }
    assert.equal(r.json.dashboard.password_set, false);
    assert.ok(!r.text.includes('password_hash'));
  });

  test('valid change is saved atomically with a backup', async () => {
    const s = await openSession(env.base);
    const r = await putSettings(env.base, { logging: { level: 'debug' } }, s);
    assert.equal(r.status, 200, r.text);
    assert.equal(r.json.restart_required, false);
    assert.match(fs.readFileSync(env.configPath, 'utf8'), /level\s*=\s*"debug"/);
    assert.match(fs.readFileSync(`${env.configPath}.bak`, 'utf8'), /level\s*=\s*"info"/);
    assert.equal((await getJson(`${env.base}/api/settings`)).json.logging.level, 'debug');
    const v = await runCsr(['--data-root', env.dir, '--config', env.configPath, 'config', 'validate']);
    assert.equal(v.code, 0, `saved config must stay valid: ${v.stderr}`);
  });

  test('invalid changes are rejected and the file is untouched', async () => {
    const s = await openSession(env.base);
    const before = fs.readFileSync(env.configPath, 'utf8');
    const bad = [
      { limits: { max_concurrent_jobs: 0 } },
      { node: { rpc_url: 'http://10.0.0.5:8114' } },
      { foo: 1 },
      { guard: { enabled: false } },
      { logging: { level: 'loud' } },
      { dashboard: { password_set: true } },
    ];
    for (const b of bad) {
      const r = await putSettings(env.base, b, s);
      assert.equal(r.status, 400, `accepted ${JSON.stringify(b)}`);
    }
    assert.equal(fs.readFileSync(env.configPath, 'utf8'), before);
  });

  test('SSE sends a status event on connect', async () => {
    const ev = await firstEvent(`${env.base}/api/events`, 'status');
    assert.ok(['Unknown', 'Synced', 'Syncing', 'Degraded', 'Unreachable'].includes(ev.r2.state));
    assert.equal(ev.network, 'testnet');
  });

  test('network change requires a restart', async () => {
    const s = await openSession(env.base);
    const r = await putSettings(env.base, { network: 'mainnet' }, s);
    assert.equal(r.status, 200, r.text);
    assert.equal(r.json.restart_required, true);
  });
});

test('public bind without password is refused at run time', async () => {
  const port = await freePort();
  const env = await setup('p4-public', { cfg: (c) => { c.dashboard.bind = `0.0.0.0:${port}`; return c; } });
  const r = await runCsr([...env.args, 'run'], { timeoutMs: 20000 });
  assert.ok(r.code !== null && r.code !== 0, 'must refuse');
  assert.match(r.stderr, /password/i);
});

test('password flow', async () => {
  const env = await setup('p4-pw');
  const pw = 'correct horse battery';
  const sp = await runCsr([...env.args, 'set-password', '--password-stdin'], { input: `${pw}\n` });
  assert.equal(sp.code, 0, sp.stderr);
  const text = fs.readFileSync(env.configPath, 'utf8');
  assert.match(text, /password_hash\s*=\s*"\$argon2id\$/);
  assert.ok(!text.includes(pw), 'plain password written to the config');
  assert.ok(fs.existsSync(`${env.configPath}.bak`));

  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  assert.equal((await fetch(`${env.base}/api/status`)).status, 401);
  assert.equal((await fetch(`${env.base}/healthz`)).status, 200);
  const s = await openSession(env.base);
  assert.equal(s.info.password_required, true);
  assert.equal(s.info.authenticated, false);
  const login = (password, sess) => fetch(`${env.base}/api/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', cookie: sess.cookie, 'x-csrf-token': sess.csrf },
    body: JSON.stringify({ password }),
  });
  assert.equal((await login('wrong password', s)).status, 401);
  const ok = await login(pw, s);
  assert.equal(ok.status, 200);
  const cookie = sessionCookie(ok) || s.cookie;
  let csrf = s.csrf;
  try { const j = await ok.json(); if (j && j.csrf_token) csrf = j.csrf_token; } catch { /* keep */ }
  assert.equal((await fetch(`${env.base}/api/status`, { headers: { cookie } })).status, 200);
  const r = await putSettings(env.base, { logging: { level: 'warn' } }, { cookie, csrf });
  assert.equal(r.status, 200, r.text);
  const set = await getJson(`${env.base}/api/settings`, { cookie });
  assert.equal(set.json.dashboard.password_set, true);
  assert.ok(!set.text.includes('argon2'));
});
