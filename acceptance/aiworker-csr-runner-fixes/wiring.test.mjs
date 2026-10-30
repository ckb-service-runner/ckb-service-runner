// csr-runner-fixes: the new pieces are wired into the running program.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));
const allSrc = () => fs.readdirSync(path.join(ROOT, 'runner', 'src'), { recursive: true })
  .filter((f) => f.endsWith('.rs')).map((f) => [f, src(f)]);

test('db.rs includes the acceptance test module', () => {
  assert.match(src('db.rs'), /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-runner-fixes\/runner_fixes_tests\.rs"\]/);
});

test('settings use needs_restart (F-RUN-009)', () => {
  assert.ok(calls(src('web/settings.rs'), 'needs_restart').length >= 1);
});

test('the history and the services page use the new data (F-RUN-011, F-RUN-012)', () => {
  assert.match(src('web/routes.rs') + src('db.rs'), /disk_free_gb/);
  assert.ok(calls(src('registry.rs'), 'service_totals').length >= 1);
  assert.doesNotMatch(src('registry.rs'), /"jobs":\s*0,/);
});

test('ping and check-in refuse an unusable monitor key (F-RUN-013)', () => {
  const ping = src('ping/inbound.rs') + src('ping/outbound.rs') + src('run.rs');
  assert.ok(calls(ping, 'monitor_key_usable').length >= 1);
});

test('the HTTP clients use proxy-free, redirect-free settings (F-RUN-014, F-SEC-017)', () => {
  for (const f of ['ping/outbound.rs', 'attest.rs', 'monitor/mod.rs', 'node/rpc.rs']) {
    assert.doesNotMatch(src(f), /Client::builder\(\)/, `${f} must use network::http_client`);
    assert.ok(calls(src(f), 'http_client').length >= 1, `${f} must call http_client`);
  }
  const t = src('network.rs');
  assert.match(t, /fn\s+http_client\b/);
  assert.match(t, /\.no_proxy\(\)/);
  assert.match(t, /Policy::none\(\)/);
});

test('the wallet works without the CSR deployment (F-RUN-017)', () => {
  assert.ok(calls(src('web/operator.rs'), 'connect_wallet').length >= 1);
  assert.ok(calls(src('run.rs') + src('wallet.rs'), 'connect_wallet').length >= 1);
});

test('logs rotate while running (F-RUN-018)', () => {
  assert.match(src('logs.rs'), /RotatingFile/);
  assert.ok(src('logs.rs').split('\n').some((l) => /RotatingFile::open\s*\(/.test(l)));
});

test('dashboard actions check for pending transactions (F-SEC-015)', () => {
  assert.ok(calls(src('web/operator.rs'), 'wallet_busy').length >= 1);
  assert.match(src('web/operator.rs'), /StatusCode::CONFLICT/);
});
