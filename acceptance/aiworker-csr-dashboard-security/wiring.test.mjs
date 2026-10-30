// csr-dashboard-security: the checks are used by the server, the unit and the docs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));

test('auth.rs includes the acceptance test module', () => {
  assert.match(src('web/auth.rs'), /#\[path = "\.\.\/\.\.\/\.\.\/acceptance\/aiworker-csr-dashboard-security\/dashboard_security_tests\.rs"\]/);
});

test('the guard uses the new checks', () => {
  const a = src('web/auth.rs');
  for (const f of ['write_needs_password', 'host_allowed', 'origin_allowed']) assert.ok(calls(a, f).length >= 1, `guard must call ${f}`);
  assert.match(a, /"password_required"/, 'a refused write answers {"error": "password_required", ...}');
});

test('login is throttled and password checks are limited', () => {
  const a = src('web/auth.rs');
  assert.match(a, /LoginLimiter/);
  assert.match(a, /Semaphore/);
  assert.match(a, /StatusCode::TOO_MANY_REQUESTS/);
  assert.match(src('run.rs'), /into_make_service_with_connect_info/);
});

test('AppState keeps sessions in the Sessions store', () => {
  assert.match(src('app.rs'), /sessions:\s*crate::web::Sessions|sessions:\s*Sessions/);
});

test('files are private and keys are wiped', () => {
  assert.match(src('main.rs') + src('run.rs'), /umask\s*\(\s*0o077\s*\)/);
  assert.match(src('identity.rs'), /key_mode_too_open\s*\(/);
  assert.match(src('identity.rs') + src('app.rs'), /[Zz]eroiz/);
  assert.match(read('runner', 'Cargo.toml'), /^zeroize\s*=/m);
  assert.match(read('config', 'csr.service'), /^LimitCORE=0$/m);
});

test('the sweep warning reaches the API, the log and the page', () => {
  assert.match(src('wallet.rs'), /"sweep_warning"/);
  assert.ok(calls(src('run.rs'), 'sweep_warning').length >= 1, 'run.rs logs the warning at start');
  const assets = fs.readdirSync(path.join(ROOT, 'runner', 'assets'), { recursive: true })
    .filter((f) => /\.js$/.test(f)).map((f) => read('runner', 'assets', f)).join('\n');
  assert.match(assets, /sweep_warning/);
});

test('the install guide covers remote access and disk encryption', () => {
  const t = read('docs', 'install.md');
  assert.match(t, /SSH tunnel/i);
  assert.match(t, /reverse proxy/i);
  assert.match(t, /full-disk encryption/i);
  assert.match(t, /password/i);
});
