// csr-docs: the runner repository's documents describe the current code.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const exists = (...p) => fs.existsSync(path.join(ROOT, ...p));
const tracked = execSync('git ls-files', { cwd: ROOT, encoding: 'utf8' }).split('\n').filter(Boolean);
const textFiles = tracked.filter((f) => /\.(md|toml|json|js|mjs|rs|sh)$/.test(f) && !f.startsWith('acceptance/'));

test('README is a current overview with the dashboard pages (F-UND-002)', () => {
  const r = read('README.md');
  assert.doesNotMatch(r.slice(0, 600), /Milestone \*\*A1\*\*/);
  assert.doesNotMatch(r, /never sends a transaction in A1/);
  for (const page of ['Overview', 'Services', 'Jobs', 'Wallet', 'Network', 'Settings']) assert.match(r, new RegExp(page), page);
});

test('roadmap and open questions are current (F-UND-003)', () => {
  assert.match(read('docs', 'roadmap.md'), /- \[x\]/i);
  assert.doesNotMatch(read('docs', 'roadmap.md'), /Phase 0.*\(now\)/);
});

test('no old repository owner (F-UND-004)', () => {
  const bad = textFiles.filter((f) => read(f).includes('psawyerberlin'));
  assert.deepEqual(bad, []);
});

test('no A7 payout cell, float or runner key cell in the current specs (F-RUN-019)', () => {
  assert.doesNotMatch(read('spec', 'host-interface.md'), /\bfloat\b/i);
  for (const f of ['registry.md', 'job-cell-standard.md']) {
    const t = read('spec', f).replace(/<!-- history -->[\s\S]*?<!-- \/history -->/g, '');
    assert.doesNotMatch(t, /anyone-can-pay payout cell|float cell|runner key cell/i, f);
  }
});

test('architecture.md matches the code (F-RUN-020)', () => {
  const t = read('docs', 'architecture.md');
  assert.doesNotMatch(t, /request_recv/);
  assert.doesNotMatch(t, /before every job/i);
});

test('old example config and unused deployment keys are gone (F-RUN-021)', () => {
  assert.ok(!exists('config', 'ckb-service-runner.example.toml'));
  const d = JSON.parse(read('deployments', 'testnet.json'));
  assert.ok(!('rpc' in d), 'deployments/testnet.json: no rpc key');
  assert.ok(!JSON.stringify(d).includes('anyone_can_pay'));
});

test('superseded decisions are marked (F-RUN-022)', () => {
  assert.match(read('docs', 'decisions.md'), /superseded by A8b/i);
});

test('the dashboard explains the check types', () => {
  const js = read('runner', 'assets', 'app.js');
  assert.match(js, /peer_connected/);
  assert.match(js, /p2p_verified/);
});

test('one place states the first transfer amount', () => {
  const r = read('README.md') + read('docs', 'install.md');
  assert.match(r, /\d{3} CKB/);
});
