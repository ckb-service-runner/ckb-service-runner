// A8 Phase 2 (devnet): job tips grow the runner wallet; the daemon sweeps automatically.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { cleanup, startCsr, getJson, waitFor } from '../a1/lib.mjs';
import { deployService, allow, createJob, startMiner } from '../a3c/jobs.mjs';
import { deploy, deposit } from '../a7/flow.mjs';
import { CKB, runner, fundRunnerCell, freshAddress, lockCapacity, wallet, registerWithConsent } from './flow.mjs';

after(cleanup);

test('an Every job runs three times; the runner wallet grows by exactly 3 tips', async () => {
  await deploy();
  const hb = await deployService('heartbeat');
  const r = await runner('a8-run', undefined, (c) => { c.runtime.enabled = true; return c; });
  // Integration (Oct 2026): the registry cell needs ≈ 540 CKB plus 61 CKB change (docs/testing.md: 700).
  await fundRunnerCell(r, 700);
  await registerWithConsent(r, await deposit(10000));
  await allow(r, hb);
  const before = (await wallet(r)).capacity_shannons;

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
  assert.equal(e.payout, undefined, 'earnings no longer report a payout cell');
  const after1 = (await wallet(r)).capacity_shannons;
  assert.equal(after1, before + 15 * CKB, 'the runner wallet grows by exactly the tips (fees come from the job)');
  assert.equal(e.wallet.capacity_shannons, after1);
  const reg = (await getJson(`${r.base}/api/registry`)).json;
  assert.equal(reg.registered, true);
  assert.equal(reg.payout_lock_hash, r.id.runner_lock_hash);
  assert.equal((await getJson(`${r.base}/api/payout`)).status, 404);
});

test('the daemon sweeps the runner wallet automatically', async () => {
  await deploy();
  const target = freshAddress('a8-auto');
  const r = await runner('a8-auto', { sweep_address: target.address, sweep_threshold_ckb: 300, sweep_amount_ckb: 200 });
  await fundRunnerCell(r, 400);
  await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const stopMiner = startMiner(500);
  try {
    await waitFor(async () => (await lockCapacity(target.args)) === 200 * CKB, 120000, 1000);
  } finally {
    await stopMiner();
  }
  const w = (await getJson(`${r.base}/api/wallet`)).json;
  assert.equal(w.address, r.id.runner_address);
  assert.equal(w.last_sweep.amount_shannons, 200 * CKB);
  assert.ok(w.capacity_shannons < 200 * CKB && w.capacity_shannons > 199 * CKB, String(w.capacity_shannons));
});

test('two runners race for one job: exactly one wallet is paid, the loser pays nothing', async () => {
  // Ported from A3c (float) to the runner wallet (Oct 2026).
  await deploy();
  const hb = await deployService('heartbeat');
  const runners = [];
  for (const name of ['a8-race-a', 'a8-race-b']) {
    const r = await runner(name, undefined, (c) => { c.runtime.enabled = true; return c; });
    await fundRunnerCell(r, 700);
    await registerWithConsent(r, await deposit(10000));
    await allow(r, hb);
    runners.push(r);
  }
  const before = [];
  for (const r of runners) before.push((await wallet(r)).capacity_shannons);
  const job = await createJob(hb.code_hash, { tipCkb: 7 });
  for (const r of runners) await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const stopMiner = startMiner(700);
  try {
    await waitFor(async () => {
      const totals = [];
      for (const r of runners) totals.push((await getJson(`${r.base}/api/earnings`)).json.tips_shannons);
      return totals.reduce((a, b) => a + b, 0) === 7 * CKB;
    }, 240000, 1000);
  } finally {
    await stopMiner();
  }
  const gains = [];
  for (let i = 0; i < runners.length; i++) gains.push((await wallet(runners[i])).capacity_shannons - before[i]);
  assert.deepEqual([...gains].sort((a, b) => a - b), [0, 7 * CKB], 'one wallet gains the tip, the other is unchanged');
  const states = [];
  for (const r of runners) {
    const jobs = (await getJson(`${r.base}/api/jobs`)).json.jobs.filter((j) => j.outpoint === job);
    states.push(jobs.map((j) => j.state));
  }
  assert.equal(states.flat().filter((s) => s === 'Settled').length, 1, JSON.stringify(states));
  for (const s of states.flat()) assert.ok(['Settled', 'Lost'].includes(s), `unexpected state ${s}`);
});
