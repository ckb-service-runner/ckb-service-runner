// csr-cell-consumed-trigger: the watch is used in job selection, the watcher task and the docs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));

test('jobs.rs includes the acceptance test module', () => {
  assert.match(src('jobs.rs'), /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-cell-consumed-trigger\/watch_tests\.rs"\]/);
});

test('selection excludes jobs whose cell is still live', () => {
  const j = src('jobs.rs');
  assert.ok(calls(j, 'watch_due').length >= 1);
  assert.match(j, /WaitingForCell/);
});

test('the watcher task collects watched out points', () => {
  assert.ok(calls(src('run.rs') + src('keeper.rs'), 'watch_outpoint').length >= 1);
});

test('the standard documents the convention', () => {
  const t = read('spec', 'job-cell-standard.md');
  assert.match(t, /CSRW/);
  assert.match(t, /OnCellConsumed/);
  assert.match(t, /cannot be (proven|checked)/i);
});
