// fix-runner-skeleton-guard: the guard is called on the real settlement path, not only in tests.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const src = (f) => fs.readFileSync(path.join(ROOT, 'runner', 'src', f), 'utf8');

test('chain.rs includes the acceptance test module', () => {
  assert.match(src('chain.rs'), /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-fix-runner-skeleton-guard\/skeleton_guard_tests\.rs"\]/);
});

test('check_skeleton_input is called outside its definition', () => {
  const calls = ['chain.rs', 'keeper.rs'].flatMap((f) => src(f).split('\n'))
    .filter((l) => /check_skeleton_input\s*\(/.test(l) && !/fn\s+check_skeleton_input/.test(l));
  assert.ok(calls.length >= 1, 'chain.rs or keeper.rs must call check_skeleton_input for every skeleton input');
});

test('the host interface spec states the rule', () => {
  const t = fs.readFileSync(path.join(ROOT, 'spec', 'host-interface.md'), 'utf8');
  assert.match(t, /skeleton input/i);
  assert.match(t, /runner-owned/i);
});
