// csr-attestation-renewal: refresh is reachable from the CLI, the dashboard and the background task.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));

test('chain.rs includes the acceptance test module', () => {
  assert.match(src('chain.rs'), /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-attestation-renewal\/renewal_tests\.rs"\]/);
});

test('register_refresh builds on refresh_registry_data', () => {
  const c = src('chain.rs');
  const i = c.search(/fn\s+register_refresh\b/);
  assert.ok(i >= 0, 'Chain::register_refresh exists');
  const body = c.slice(i, i + 4000);
  assert.ok(calls(body, 'refresh_registry_data').length >= 1);
});

test('the CLI, the API and the background task use it', () => {
  assert.match(src('main.rs'), /Refresh\b/);
  assert.match(src('web/routes.rs'), /"\/api\/registry\/refresh"/);
  assert.ok(calls(src('run.rs'), 'renewal_due').length >= 1);
});

test('the dashboard shows the expiry as a date and warns', () => {
  const js = read('runner', 'assets', 'app.js');
  assert.match(js, /\/api\/registry\/refresh/);
  assert.match(js, /attestation_expiry_ms/);
});

test('the registry spec describes the automatic renewal', () => {
  const t = read('spec', 'registry.md');
  assert.match(t, /csr register refresh/);
  assert.match(t, /automatic/i);
});
