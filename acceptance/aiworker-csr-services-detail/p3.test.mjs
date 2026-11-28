// csr-services-detail, phase 3: documentation.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('decision D56 and changelog entry', () => {
  assert.match(read('docs', 'decisions.md'), /D56/);
  const top = read('CHANGELOG.md').slice(0, 6000);
  assert.match(top, /Verify binary/);
  assert.match(top, /Unreleased/);
});

test('roadmap lists the work with ticks', () => {
  assert.match(read('docs', 'roadmap.md'), /\[x\][^\n]*(Verify binary|service details)/i);
});

test('security model says what Verify binary reads', () => {
  const s = read('docs', 'security-model.md');
  assert.match(s, /Verify binary/);
  assert.match(s, /own node/);
});

test('publishers are told which manifest fields operators see', () => {
  const p = read('docs', 'publishing-a-service.md');
  assert.match(p, /detail panel|Details/);
  assert.match(p, /Verify binary/);
  assert.match(p, /own claim|publisher's claim/);
});
