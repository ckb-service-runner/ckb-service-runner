// A8 Phase 1: the stake owner consents by a signed message (Neuron "Sign message" format); the registry
// owner may be any lock (CSR uses the runner's own lock).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT } from '../a1/lib.mjs';

const read = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');

test('consent module is shared by the contract and the tests', () => {
  const c = read('contracts/csr-registry/src/consent.rs');
  assert.match(c, /CSR-STAKE-CONSENT-v1/);
  assert.match(c, /Nervos Message:/);
  const main = read('contracts/csr-registry/src/main.rs');
  assert.match(main, /mod consent;/);
  for (const code of [21, 22, 23]) assert.match(main, new RegExp(`:\\s*i8\\s*=\\s*${code};`), `error code ${code}`);
  const tests = fs.readdirSync(path.join(ROOT, 'contracts', 'csr-contract-tests'), { recursive: true })
    .filter((f) => f.endsWith('.rs')).map((f) => read(path.join('contracts', 'csr-contract-tests', f))).join('\n');
  assert.match(tests, /csr-registry\/src\/consent\.rs/, 'the tests must include the contract\'s consent.rs via #[path]');
  assert.ok(tests.includes('e3425937793f1e3c6de997e908df0c8ac00efdfd8db48948adca4d4e9a4b51c2'), 'consent text digest vector');
  assert.ok(tests.includes('c0fee686c87c2f398a83ef0ab8ddcc462b0acd552411674b3d1cf8e9ce6a86563204ffd3e2b82a6dfa9be40c58e1d670faf8ea8656db45e1d05d4265912877f401'),
    'Neuron signature vector');
});

test('the job lock still pays tips to payout_lock_hash', () => {
  assert.match(read('contracts/csr-job-lock/src/main.rs'), /payout_lock_hash/);
});

test('spec/registry.md documents the consent and the runner-owned registration', () => {
  const s = read('spec/registry.md');
  for (const w of ['CSR-STAKE-CONSENT-v1', 'Nervos Message:', 'output_type', '| 21 |', '| 22 |', '| 23 |', 'stake owner',
    'runner wallet',
    'CSR-STAKE-CONSENT-v1|testnet|0x5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a:3|0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27|QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB']) {
    assert.ok(s.includes(w), `spec/registry.md must contain ${w}`);
  }
  assert.match(read('docs/decisions.md'), /D-A8-1/);
});
