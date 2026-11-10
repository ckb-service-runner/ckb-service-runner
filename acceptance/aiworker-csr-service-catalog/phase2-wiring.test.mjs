import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);

test('catalog.rs includes the service ID vectors', () => {
  assert.match(src('catalog.rs'),
    /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-service-catalog\/service_id_tests\.rs"\]/);
});

test('catalog module exists and reads the chain, not the monitor', () => {
  const c = src('catalog.rs');
  assert.match(c, /get_cells|cells_by_type|search_key/);
  assert.doesNotMatch(c, /monitor/i);
});

test('routes expose the catalog endpoints', () => {
  const r = read('runner', 'src', 'web', 'routes.rs');
  for (const p of ['/api/catalog', '/api/catalog/refresh', '/api/catalog/{type_id}/add', '/api/catalog/seen'])
    assert.ok(r.includes(`"${p}"`), p);
});

test('keeper passes data cells and args to the virtual transaction', () => {
  const k = src('keeper.rs');
  assert.doesNotMatch(k, /VirtualTx::new\(\s*elf\.clone\(\),\s*vec!\[\],\s*vec!\[\]/);
});

test('config and deployment know the catalog', () => {
  assert.match(read('config', 'csr.example.toml'), /\[catalog\]/);
  assert.match(src('config.rs'), /scan_hours/);
  assert.match(src('chain.rs'), /service_manifest/);
});
