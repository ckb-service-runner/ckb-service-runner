// A7 Phase 4: keeper jobs pay their tips into the anyone-can-pay payout cell.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { cleanup, startCsr, getJson, waitFor } from '../a1/lib.mjs';
import { csrJson, liveCell } from '../a3b/devnet.mjs';
import { fundRunnerCell } from '../a3b/runner.mjs';
import { deployService, allow, createJob, startMiner } from '../a3c/jobs.mjs';
import { CKB, deploy, runner, deposit, createPayout, register, payoutArgs } from './flow.mjs';

after(cleanup);

test('an Every job runs three times; the payout cell grows by exactly 3 tips', async () => {
  await deploy();
  const hb = await deployService('heartbeat');
  const args = payoutArgs('a7-run');
  const r = await runner('a7-run', args, (c) => { c.runtime.enabled = true; return c; });
  await fundRunnerCell(r, 500);
  await createPayout(r, 100);
  const stake = await deposit(10000);
  await register(r, stake);
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
  assert.equal(e.tips_shannons, 15 * CKB);
  assert.equal(e.fees_shannons, 0);
  assert.equal(e.float, undefined, 'earnings must not report a float any more');
  const pay = (await csrJson([...r.args, 'payout', 'status'])).json;
  assert.equal(pay.capacity_shannons, 115 * CKB, 'the payout cell must grow by exactly the tips');
  assert.equal(e.payout.capacity_shannons, pay.capacity_shannons);
  const [ptx, pidx] = pay.outpoint.split(':');
  assert.equal((await liveCell(ptx, Number(pidx))).cell.output.lock.args, args);
  const rc = (await csrJson([...r.args, 'runner-cell', 'status'])).json;
  assert.equal(rc.capacity_shannons, runnerBefore, 'the runner key cell must not pay fees');

  // dashboard API
  const ap = (await getJson(`${r.base}/api/payout`)).json;
  assert.equal(ap.found, true);
  assert.equal(ap.capacity_shannons, 115 * CKB);
  const reg = (await getJson(`${r.base}/api/registry`)).json;
  assert.equal(reg.registered, true);
  assert.equal(reg.stake.status, 'deposited');
  assert.equal(reg.payout_lock_hash, pay.lock_hash);
  assert.equal((await getJson(`${r.base}/api/float`)).status, 404);
});

test('two runners race for one job: exactly one payout cell is paid', async () => {
  await deploy();
  const hb = await deployService('heartbeat');
  const runners = [];
  for (const name of ['a7-race-a', 'a7-race-b']) {
    const r = await runner(name, payoutArgs(name), (c) => { c.runtime.enabled = true; return c; });
    await fundRunnerCell(r, 500);
    await createPayout(r, 100);
    await register(r, await deposit(10000));
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
      return totals.reduce((a, b) => a + b, 0) === 7 * CKB;
    }, 120000, 1000);
  } finally {
    await stopMiner();
  }
  const pays = [];
  for (const r of runners) pays.push((await csrJson([...r.args, 'payout', 'status'])).json.capacity_shannons);
  assert.deepEqual([...pays].sort((a, b) => a - b), [100 * CKB, 107 * CKB]);
});

test('without payout.lock_args the keeper runs nothing and says why', async () => {
  await deploy();
  const r = await runner('a7-nopayout', '', (c) => { c.runtime.enabled = true; return c; });
  await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const e = (await getJson(`${r.base}/api/earnings`)).json;
  assert.deepEqual(e.payout, { capacity_shannons: null });
});
