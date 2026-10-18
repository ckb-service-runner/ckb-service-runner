import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { cleanup, startCsr, getJson, waitFor } from '../a1/lib.mjs';
import { csrJson } from '../a3b/devnet.mjs';
import { devnetRunner, fundRunnerCell, createFloat, register } from '../a3b/runner.mjs';
import { deployContracts, deployService, allow, createJob, startMiner } from './jobs.mjs';

after(cleanup);

test('an Every job runs three times; the float grows by exactly 3 tips', async () => {
  await deployContracts();
  const hb = await deployService('heartbeat');
  const r = await devnetRunner('a3c-run', (c) => { c.runtime.enabled = true; return c; });
  await fundRunnerCell(r, 500);
  await createFloat(r, 1000);
  await register(r);
  await allow(r, hb);
  const runnerBefore = (await csrJson([...r.args, 'runner-cell', 'status'])).json.capacity_shannons;

  await createJob(hb.code_hash, { tipCkb: 5, trigger: 'every:2', runs: 3 });
  await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const stopMiner = startMiner(500);
  try {
    await waitFor(async () => {
      const e = (await getJson(`${r.base}/api/earnings`)).json;
      return e && e.settlements && e.settlements.length >= 3 && e;
    }, 240000, 1000);
  } finally {
    await stopMiner();
  }
  const e = (await getJson(`${r.base}/api/earnings`)).json;
  assert.equal(e.settlements.length, 3);
  assert.equal(e.tips_shannons, 15 * 1e8);
  assert.equal(e.fees_shannons, 0);
  const fl = (await csrJson([...r.args, 'float', 'status'])).json;
  assert.equal(fl.capacity_shannons, 1015 * 1e8, 'float must grow by exactly the tips');
  assert.equal(e.float.capacity_shannons, fl.capacity_shannons);
  const rc = (await csrJson([...r.args, 'runner-cell', 'status'])).json;
  assert.equal(rc.capacity_shannons, runnerBefore, 'the runner key cell must not pay fees');
  const jobs = (await getJson(`${r.base}/api/jobs`)).json.jobs;
  assert.equal(jobs.filter((j) => j.state === 'Settled').length, 3);
  for (const j of jobs.filter((x) => x.state === 'Settled')) assert.match(j.result_tx, /^0x[0-9a-f]{64}$/);
});
