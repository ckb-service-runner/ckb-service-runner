// csr-mainnet-dashboard, phase 3: documents for operators (D29–D39).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

function section(text, headingRe) {
  const lines = text.split('\n');
  const i = lines.findIndex((l) => /^#{1,4} /.test(l) && headingRe.test(l));
  if (i < 0) return null;
  const level = lines[i].match(/^#+/)[0].length;
  let j = i + 1;
  while (j < lines.length && !(new RegExp(`^#{1,${level}} `).test(lines[j]))) j++;
  return lines.slice(i, j).join('\n');
}

test('README: what it costs an operator', () => {
  const s = section(read('README.md'), /cost/i);
  assert.ok(s, 'README needs a section whose heading contains "cost"');
  assert.match(s, /10,000 CKB/);
  assert.match(s, /700 CKB/);
  assert.match(s, /Nervos DAO/);
  assert.match(s, /539 CKB/);
  assert.match(s, /fee/i);
  assert.match(s, /back|recover|return/i);           // everything except fees comes back
  assert.match(s, /dashboard/i);                     // the dashboard alone needs no CKB
});

test('README says mainnet is dashboard-only until the Phase 2 gate', () => {
  const r = read('README.md');
  assert.match(r, /mainnet/i);
  assert.match(r, /(4 months|120 days)/);
  assert.match(r, /20 (ranked )?runners/);
  assert.match(r, /\b1\.x\b|\bv1\b|1\.0/);
});

test('install guide: one instance per network', () => {
  const t = read('docs', 'install.md');
  for (const s of ['csr@testnet', 'csr@mainnet', 'csr-testnet.toml', 'csr-mainnet.toml', '8130', '8140', '8131', '8141',
    'confirm-mainnet', 'csr-update@', 'peer_url']) {
    assert.ok(t.includes(s), s);
  }
  assert.match(t, /csr\.toml[\s\S]{0,300}(migrat|moved|renamed)/i);   // what happens to an old single install
  assert.match(t, /--rpc/);
});

test('quick start covers mainnet nodes', () => {
  const q = read('docs', 'quickstart.md');
  assert.match(q, /mainnet/i);
  assert.match(q, /8140/);
  assert.doesNotMatch(q, /testnet only/i);
});

test('no document still says the installer is testnet-only or that a toggle changes the network', () => {
  for (const f of ['README.md', 'docs/install.md', 'docs/quickstart.md']) {
    const t = read(...f.split('/'));
    assert.doesNotMatch(t, /installer (sets up|is for) testnet only/i, f);
    assert.doesNotMatch(t, /change the network in (the )?settings/i, f);
  }
});

test('open questions: Neuron answered as far as known', () => {
  const o = read('docs', 'open-questions.md');
  assert.match(o, /Neuron/);
  assert.match(o, /light client/i);
});
