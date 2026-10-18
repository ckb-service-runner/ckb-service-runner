import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runCsr, cleanup } from '../a1/lib.mjs';
import { csrJson } from './devnet.mjs';
import { devnetRunner, fundRunnerCell, createFloat, signAndSubmit } from './runner.mjs';

after(cleanup);

test('runner key cell, float create, top-up and withdraw', async () => {
  const r = await devnetRunner('a3b-float');
  assert.equal((await csrJson([...r.args, 'float', 'status'])).json.found, false);
  await fundRunnerCell(r, 200);
  const rc = (await csrJson([...r.args, 'runner-cell', 'status'])).json;
  assert.equal(rc.found, true);
  assert.equal(rc.capacity_shannons, 200 * 1e8);

  const owner = await createFloat(r, 1000, 1000000);
  let fl = (await csrJson([...r.args, 'float', 'status'])).json;
  assert.equal(fl.found, true);
  assert.equal(fl.capacity_shannons, 1000 * 1e8);
  assert.equal(fl.max_fee_shannons, 1000000);
  assert.equal(fl.owner_lock_hash, owner.lock_hash);
  assert.equal(fl.runner_lock_hash, r.id.runner_lock_hash);

  const top = path.join(r.dir, 'topup.json');
  assert.equal((await runCsr([...r.args, 'float', 'prepare-topup', '--ckb', '500', '--out', top], { timeoutMs: 120000 })).code, 0);
  await signAndSubmit(r, top);
  fl = (await csrJson([...r.args, 'float', 'status'])).json;
  assert.equal(fl.capacity_shannons, 1500 * 1e8);

  const wd = path.join(r.dir, 'withdraw.json');
  assert.equal((await runCsr([...r.args, 'float', 'prepare-withdraw', '--ckb', '300', '--out', wd], { timeoutMs: 120000 })).code, 0);
  await signAndSubmit(r, wd);
  fl = (await csrJson([...r.args, 'float', 'status'])).json;
  assert.equal(fl.capacity_shannons, 1200 * 1e8);
});

test('an unsigned transaction is rejected by the node', async () => {
  const r = await devnetRunner('a3b-unsigned');
  const owner = await (await import('./devnet.mjs')).csrDev(['owner-lock-args']);
  const out = path.join(r.dir, 'unsigned.json');
  const p = await runCsr([...r.args, 'float', 'prepare-create', '--ckb', '1000', '--owner-lock-args', owner.json.lock_args, '--out', out], { timeoutMs: 120000 });
  assert.equal(p.code, 0, p.stderr);
  assert.ok(fs.existsSync(out));
  const sub = await runCsr([...r.args, 'tx', 'submit', out, '--json'], { timeoutMs: 60000 });
  assert.notEqual(sub.code, 0, 'an unsigned transaction must not be accepted');
});
