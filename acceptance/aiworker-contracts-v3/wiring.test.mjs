// contracts-v3: the vectors are used by the contract tests, the specs are updated, the new contract exists.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const exists = (...p) => fs.existsSync(path.join(ROOT, ...p));

test('the config type contract is a workspace member', () => {
  assert.ok(exists('contracts', 'csr-config-type', 'src', 'main.rs'));
  assert.match(read('contracts', 'Cargo.toml'), /"csr-config-type"/);
  assert.match(read('scripts', 'build-contracts.sh'), /csr-config-type/);
});

test('the schema has RegistryV3 and AttestationBodyV2', () => {
  const m = read('contracts', 'schemas', 'csr.mol');
  assert.match(m, /table RegistryV3/);
  assert.match(m, /table AttestationBodyV2/);
});

test('the contract tests read the shared vector', () => {
  const dir = path.join(ROOT, 'contracts', 'csr-contract-tests');
  const all = fs.readdirSync(dir, { recursive: true }).filter((f) => f.endsWith('.rs'))
    .map((f) => fs.readFileSync(path.join(dir, f), 'utf8')).join('\n');
  assert.match(all, /registry-v3\.json/);
});

test('the specs describe v3, attestation v2, the cap, the clock rule and the refund index', () => {
  const r = read('spec', 'registry.md');
  assert.match(r, /CSR-ATTEST-v2/);
  assert.match(r, /RegistryV3|Registry v3/);
  assert.match(r, /8 days/);
  assert.match(r, /csr-config-type/);
  assert.match(r, /7 days/, 'the stake window of decision D22 C is stated');
  const j = read('spec', 'job-cell-standard.md');
  assert.match(j, /same index/i);
  assert.match(j, /one group|same service/i, 'the one-job-per-service-group limit is documented (F-CON-007)');
});
