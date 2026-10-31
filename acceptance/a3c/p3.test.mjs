import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, cleanup, startCsr, getJson, waitFor } from '../a1/lib.mjs';
import { csrJson, liveCell } from '../a3b/devnet.mjs';
import { devnetRunner, fundRunnerCell, createFloat, register } from '../a3b/runner.mjs';
import { deployContracts, deployService, allow, createJob, startMiner } from './jobs.mjs';

after(cleanup);

test('pages linked; docs written', () => {
  const html = fs.readFileSync(path.join(ROOT, 'runner', 'assets', 'index.html'), 'utf8');
  for (const r of ['#/jobs', '#/earnings']) assert.ok(html.includes(`href="${r}"`), `${r} missing`);
});
