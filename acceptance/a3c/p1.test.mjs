import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { cleanup } from '../a1/lib.mjs';
import { csrJson, tipNumber } from '../a3b/devnet.mjs';
import { devnetRunner } from '../a3b/runner.mjs';
import { deployContracts, deployService, allow, createJob } from './jobs.mjs';

after(cleanup);

test('plan orders eligible jobs by tip rate and explains exclusions', async () => {
  await deployContracts();
  const hb = await deployService('heartbeat');
  const kv = await deployService('kvtest');
  const r = await devnetRunner('a3c-plan', (c) => {
    c.runtime.enabled = true;
    c.runtime.min_ckb_per_mcycle = 0.5;
    c.limits.max_cycles_per_job = 70000000;
    return c;
  });
  await allow(r, hb);

  const j1 = await createJob(hb.code_hash, { tipCkb: 10 });
  const j2 = await createJob(hb.code_hash, { tipCkb: 50 });
  const j3 = await createJob(hb.code_hash, { tipCkb: 30 });
  const lowRate = await createJob(hb.code_hash, { tipCkb: 2 });
  const tooBig = await createJob(hb.code_hash, { tipCkb: 100, maxCycles: 100000000 });
  const foreign = await createJob(kv.code_hash, { tipCkb: 100 });
  const notDue = await createJob(hb.code_hash, { tipCkb: 40, trigger: `at-block:${(await tipNumber()) + 1000}` });

  const plan = await csrJson([...r.args, 'jobs', 'plan']);
  assert.equal(plan.code, 0, plan.stderr);
  const p = plan.json;
  assert.deepEqual(p.selected.map((s) => s.outpoint), [j2, j3, j1]);
  assert.equal(p.selected[0].tip_rate, 5);
  assert.equal(p.selected[0].tip_shannons, 50 * 1e8);
  const reason = (op) => (p.excluded.find((e) => e.outpoint === op) || {}).reason;
  assert.equal(reason(lowRate), 'below_min_rate');
  assert.equal(reason(tooBig), 'above_max_cycles');
  assert.equal(reason(foreign), 'not_allowlisted');
  assert.equal(reason(notDue), 'trigger_not_due');
  assert.equal(p.capacity, 2);
});
