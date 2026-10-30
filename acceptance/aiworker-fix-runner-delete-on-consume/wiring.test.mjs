// fix-runner-delete-on-consume: `csr run` applies the rules, not only the CLI.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const SRC = path.join(ROOT, 'runner', 'src');
const walk = (d) => fs.readdirSync(d, { withFileTypes: true })
  .flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));

test('storage/mod.rs includes the acceptance test module', () => {
  const t = fs.readFileSync(path.join(SRC, 'storage', 'mod.rs'), 'utf8');
  assert.match(t, /#\[path = "\.\.\/\.\.\/\.\.\/acceptance\/aiworker-fix-runner-delete-on-consume\/consumed_watch_tests\.rs"\]/);
});

test('a module outside storage calls apply_consumed', () => {
  const users = walk(SRC).filter((f) => f.endsWith('.rs') && !f.includes(`${path.sep}storage${path.sep}`))
    .filter((f) => /apply_consumed\s*\(/.test(fs.readFileSync(f, 'utf8')));
  assert.ok(users.length >= 1, 'the background task in csr run must call storage::apply_consumed');
});

test('the docs no longer say the rules need the CLI', () => {
  const t = fs.readFileSync(path.join(ROOT, 'docs', 'architecture.md'), 'utf8');
  assert.match(t, /delete_on_consume/);
  assert.match(t, /get_live_cell/);
});
