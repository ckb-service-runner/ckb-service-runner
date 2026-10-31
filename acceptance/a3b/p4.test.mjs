import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runCsr, cleanup } from '../a1/lib.mjs';
import { csrDev, csrJson } from './devnet.mjs';
import { devnetRunner, fundRunnerCell, createFloat, register } from './runner.mjs';

after(cleanup);

test('docs', () => {
  for (const f of ['docs/testing.md', 'docs/milestones/A3b.md']) assert.ok(fs.existsSync(f), `${f} missing`);
  assert.match(fs.readFileSync('docs/testing.md', 'utf8'), /testnet/i);
});
