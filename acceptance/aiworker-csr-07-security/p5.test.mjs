// csr-07-security, phase 5: security model, SECURITY.md, roadmap.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('security-model.md has the required sections in order', () => {
  const d = read('docs', 'security-model.md');
  const heads = [...d.matchAll(/^## (.+)$/gm)].map((m) => m[1].trim());
  const want = ['What CSR reads', 'What CSR never touches', 'What leaves the machine', 'Privileges by platform', 'Verifying a download', 'Known gaps'];
  let at = -1;
  for (const w of want) {
    const i = heads.indexOf(w);
    assert.ok(i > at, `heading "${w}" missing or out of order (found: ${heads.join(' | ')})`);
    at = i;
  }
  for (const re of [/NT SERVICE\\CSR-/, /SeChangeNotifyPrivilege/, /127\.0\.0\.1/, /Neuron/, /SHA256SUMS/, /gh attestation verify/,
    /secret_key|p2p_key_file/, /Authenticode/, /audit/i, /ckb\.toml/, /systemd/i, /inbound/i]) {
    assert.match(d, re, String(re));
  }
  assert.ok(d.length > 3500, 'a real document, not a stub');
  assert.doesNotMatch(d, /0x[0-9a-fA-F]{64}(?![0-9a-fA-F])(?=[^\n]*secret)/i, 'no key material');
});

test('SECURITY.md status and link', () => {
  const s = read('SECURITY.md');
  assert.match(s, /security-model\.md/);
  assert.match(s, /mainnet[^\n]*dashboard/i);
  assert.match(s, /Report a vulnerability/);
  assert.doesNotMatch(s, /No release is supported yet/);
});

test('README links the security model', () => {
  const r = read('README.md');
  assert.match(r, /docs\/security-model\.md/);
});

test('roadmap: eight items ticked, signing and audit still open', () => {
  const r = read('docs', 'roadmap.md');
  const m = r.match(/^## Runner release 0\.7: security hardening[^\n]*\n([\s\S]*?)(?=^## )/m);
  assert.ok(m, 'section not found');
  assert.match(r, /^## Runner release 0\.7: security hardening \(in progress\)/m);
  const items = m[1].split(/\r?\n/).filter((l) => /^- \[[ x]\]/.test(l));
  assert.equal(items.length, 10, `ten items expected, found ${items.length}`);
  const done = items.filter((l) => /^- \[x\]/.test(l));
  assert.equal(done.length, 8, done.length + ' ticked');
  const open = items.filter((l) => /^- \[ \]/.test(l)).join('\n');
  assert.match(open, /Authenticode/);
  assert.match(open, /audit/i);
  // the other sections are untouched
  assert.match(r, /^## Runner release 0\.6\.1 \(released\)/m);
  assert.match(r, /^## Phase 2/m);
});

test('changelog completed', () => {
  const c = read('CHANGELOG.md');
  const sec = c.split(/^## Unreleased/m)[1].split(/^## /m)[0];
  assert.match(sec, /security-model/);
  assert.match(sec, /SECURITY\.md/);
});

test('mainnet runtime lock is still closed', () => {
  const m = read('runner', 'src', 'network.rs').match(/fn mainnet_runtime_unlocked\s*\(\s*\)\s*->\s*bool\s*\{([\s\S]{0,200}?)\}/);
  assert.ok(m);
  assert.match(m[1].replace(/\/\/.*$/mg, ''), /^\s*false\s*$/);
});
