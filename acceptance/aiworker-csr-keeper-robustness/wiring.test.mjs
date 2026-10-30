// csr-keeper-robustness: the new functions are used on the real paths, not only in tests.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const src = (f) => fs.readFileSync(path.join(ROOT, 'runner', 'src', f), 'utf8');
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));
const fnBody = (text, name) => {
  const i = text.search(new RegExp(`fn\\s+${name}\\b`));
  assert.ok(i >= 0, `fn ${name} not found`);
  return text.slice(i, i + 1500);
};

test('keeper.rs includes the acceptance test module', () => {
  assert.match(src('keeper.rs'), /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-keeper-robustness\/keeper_robustness_tests\.rs"\]/);
});

test('a failed batch is cleaned up (F-FUN-001)', () => {
  assert.ok(calls(src('keeper.rs'), 'abort_batch').length >= 1, 'keeper.rs must call abort_batch when execute or submit fails');
});

test('runs are filtered in SQL and indexed by state (F-FUN-003)', () => {
  const db = src('db.rs');
  assert.match(fnBody(db, 'runs_in'), /state IN/i, 'runs_in must filter with WHERE state IN (...)');
  assert.match(db, /CREATE INDEX IF NOT EXISTS \w+ ON job_runs \(state/);
  assert.ok(calls(src('run.rs'), 'prune_runs').length >= 1, 'run.rs must prune old runs periodically');
});

test('background tasks are supervised (F-FUN-004)', () => {
  assert.match(src('run.rs'), /JoinSet/, 'run.rs must keep the task handles in a tokio JoinSet');
});

test('a pending sweep is given up and survives a restart (F-FUN-005)', () => {
  const run = src('run.rs');
  assert.ok(calls(run, 'sweep_given_up').length >= 1);
  assert.match(run, /"sweep_pending"/, 'the pending sweep is stored in the settings table under "sweep_pending"');
});

test('job discovery does not mark runs Lost from an incomplete list (F-FUN-007)', () => {
  assert.ok(calls(src('keeper.rs'), 'lost_runs').length >= 1);
  assert.match(src('chain.rs'), /complete/);
});

test('the indexer may lag like R2 allows (F-FUN-008)', () => {
  assert.ok(calls(fnBody(src('chain.rs'), 'wait_indexer'), 'indexer_ready').length >= 1);
});

test('an expired attestation stops runs (F-RUN-015)', () => {
  assert.ok(calls(src('chain.rs'), 'attestation_usable').length >= 1 || calls(src('keeper.rs'), 'attestation_usable').length >= 1);
});

test('the job detail shows the attempt number', () => {
  assert.match(fnBody(src('keeper.rs'), 'run_detail'), /"attempt"/);
});

test('the run lifecycle is documented (F-UND-001)', () => {
  const t = fs.readFileSync(path.join(ROOT, 'spec', 'job-cell-standard.md'), 'utf8');
  assert.match(t, /^#+ .*Run lifecycle/m);
  assert.match(t, /500 blocks/);
  assert.match(t, /30 blocks/);
  assert.match(t, /three/i);
  assert.match(t, /more than once/i);
  assert.match(t, /not (yet )?started/i, 'the pause rule of D4 A');
  assert.doesNotMatch(src('keeper.rs'), /Running\s*(⇄|<->)\s*Paused/);
});
