import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('schema defines ServiceManifestV1', () => {
  const m = read('contracts', 'schemas', 'csr.mol');
  assert.match(m, /table ServiceManifestV1/);
  for (const f of ['code_hash', 'code_cell', 'data_hashes', 'data_cells', 'args', 'name', 'version',
    'description', 'url', 'source', 'language', 'min_csr', 'deprecated']) assert.match(m, new RegExp(`\\b${f}\\s*:`));
});

test('the contract is a workspace member and built by the build script', () => {
  assert.match(read('contracts', 'Cargo.toml'), /"csr-service-manifest"/);
  assert.match(read('scripts', 'build-contracts.sh'), /csr-service-manifest/);
});

test('the standard documents binding, service ID and the no-update guarantee', () => {
  const s = read('spec', 'service-manifest.md');
  assert.match(s, /CSR-SERVICE-v1/);
  assert.match(s, /publisher/i);
  assert.match(s, /no runner update/i);
  assert.match(read('spec', 'job-cell-standard.md'), /service-manifest\.md/);
});

test('existing contracts are unchanged in behaviour (job type args stay 32 bytes)', () => {
  assert.match(read('contracts', 'csr-job-type', 'src', 'main.rs'), /args\.len\(\) != 32/);
});
