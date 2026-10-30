// csr-sandbox-storage: migration at start, cycle charges in the host, VM memory fixed in the UI and docs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));

test('storage/mod.rs includes the acceptance test module', () => {
  assert.match(src('storage/mod.rs'), /#\[path = "\.\.\/\.\.\/\.\.\/acceptance\/aiworker-csr-sandbox-storage\/sandbox_storage_tests\.rs"\]/);
});

test('csr run migrates version 1 storage before the keeper starts', () => {
  assert.ok(calls(src('run.rs'), 'migrate_v1').length >= 1);
});

test('host calls are charged by size and node calls extra (F-SEC-008)', () => {
  const h = src('runtime/host.rs');
  assert.ok(calls(h, 'data_cycles').length >= 3, 'data_cycles is charged in the data-moving calls');
  assert.ok(h.split('\n').filter((l) => /NODE_CALL_CYCLES/.test(l) && !/const\s+NODE_CALL_CYCLES/.test(l)).length >= 2,
    'NODE_CALL_CYCLES is charged in chain_read and cells_query');
  assert.ok(h.split('\n').filter((l) => /MAX_DEBUG_SCAN/.test(l) && !/const\s+MAX_DEBUG_SCAN/.test(l)).length >= 1,
    'the debug scan is capped with MAX_DEBUG_SCAN');
  assert.match(h, /delete_on_consume[\s\S]{0,400}QuotaExceeded|QuotaExceeded[\s\S]{0,400}delete_on_consume/);
});

test('the VM memory is not offered as a setting (F-RUN-010)', () => {
  assert.doesNotMatch(read('runner', 'assets', 'app.js'), /\['limits\.max_vm_memory_mb'/);
  assert.match(read('config', 'csr.example.toml'), /max_vm_memory_mb\s*=\s*4\s*#.*(fixed|only 4)/i);
});

test('the host interface documents costs, rules and the storage format', () => {
  const t = read('spec', 'host-interface.md');
  assert.match(t, /NODE_CALL_CYCLES|100[ ,.]?000 cycles/);
  assert.match(t, /256 rules/);
  assert.match(t, /storage-v2|format version 2|version 2 format/i);
});
