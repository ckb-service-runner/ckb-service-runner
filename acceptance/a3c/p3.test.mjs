import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, cleanup, startCsr, getJson, waitFor } from '../a1/lib.mjs';
import { csrJson, liveCell } from '../a3b/devnet.mjs';
import { devnetRunner, fundRunnerCell, createFloat, register } from '../a3b/runner.mjs';
import { deployContracts, deployService, allow, createJob, startMiner } from './jobs.mjs';

after(cleanup);

test('two runners race for one job: exactly one is paid, the loser pays nothing', async () => {
  await deployContracts();
  const hb = await deployService('heartbeat');
  const runners = [];
  for (const name of ['a3c-race-a', 'a3c-race-b']) {
    const r = await devnetRunner(name, (c) => { c.runtime.enabled = true; return c; });
    await fundRunnerCell(r, 500);
    await createFloat(r, 1000);
    await register(r);
    await allow(r, hb);
    runners.push(r);
  }
  const job = await createJob(hb.code_hash, { tipCkb: 7 });
  for (const r of runners) await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const stopMiner = startMiner(700);
  try {
    const [tx, idx] = job.split(':');
    await waitFor(async () => (await liveCell(tx, Number(idx))).status !== 'live', 180000, 1000);
    await waitFor(async () => {
      const totals = [];
      for (const r of runners) totals.push((await getJson(`${r.base}/api/earnings`)).json.tips_shannons);
      return totals.reduce((a, b) => a + b, 0) === 7 * 1e8;
    }, 120000, 1000);
  } finally {
    await stopMiner();
  }
  const floats = [];
  for (const r of runners) floats.push((await csrJson([...r.args, 'float', 'status'])).json.capacity_shannons);
  assert.deepEqual([...floats].sort((a, b) => a - b), [1000 * 1e8, 1007 * 1e8]);
  const states = [];
  for (const r of runners) {
    const jobs = (await getJson(`${r.base}/api/jobs`)).json.jobs.filter((j) => j.outpoint === job);
    states.push(jobs.map((j) => j.state));
  }
  assert.equal(states.flat().filter((s) => s === 'Settled').length, 1, JSON.stringify(states));
  for (const s of states.flat()) assert.ok(['Settled', 'Lost'].includes(s), `unexpected state ${s}`);
});

test('pages linked; docs written', () => {
  const html = fs.readFileSync(path.join(ROOT, 'runner', 'assets', 'index.html'), 'utf8');
  for (const r of ['#/jobs', '#/earnings']) assert.ok(html.includes(`href="${r}"`), `${r} missing`);
  assert.ok(fs.existsSync(path.join(ROOT, 'docs', 'milestones', 'A3c.md')));
});
