import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('identity.rs includes the acceptance module', () => {
  assert.match(read('runner', 'src', 'identity.rs'),
    /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-runner-id\/runner_id_tests\.rs"\]/);
});

test('status API carries the runner ID and monitor links', () => {
  const r = read('runner', 'src', 'web', 'routes.rs') + read('runner', 'src', 'app.rs');
  assert.match(r, /runner_id/);
  assert.match(r, /runner_links/);
  assert.match(r, /\?runner=/);
});

test('dashboard shows the Runner ID with a monitor link', () => {
  const a = read('runner', 'assets', 'app.js');
  assert.match(a, /Runner ID/);
  assert.match(a, /runner_links/);
  assert.match(a, /View on monitor/);
});

test('roadmap and changelog updated', () => {
  assert.match(read('docs', 'roadmap.md'), /\[x\][^\n]*Runner ID/);
  assert.match(read('CHANGELOG.md'), /Runner ID/);
});
