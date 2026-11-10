import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const read = (...p) => fs.readFileSync(path.join(process.cwd(), ...p), 'utf8');

test('integration guide covers the pattern and the failure modes', () => {
  const g = read('docs', 'integrating-an-app.md');
  assert.match(g, /CrowdCell/);
  assert.match(g, /AtBlock/);
  assert.match(g, /empty skeleton/i);
  assert.match(g, /refund_lock_hash/);
  assert.match(g, /csr-job\.mjs/);
  assert.match(g, /approve/i);
});

test('guide is linked and recorded', () => {
  assert.match(read('README.md'), /integrating-an-app\.md/);
  assert.match(read('spec', 'job-cell-standard.md'), /integrating-an-app\.md/);
  assert.match(read('CHANGELOG.md'), /csr-job\.mjs|job builder/i);
  assert.match(read('docs', 'roadmap.md'), /CrowdCell/);
});
