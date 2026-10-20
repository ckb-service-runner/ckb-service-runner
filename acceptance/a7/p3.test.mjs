// A7 Phase 3: registration with a DAO stake and a payout cell; `register attest --stake`.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { tmpDir, freePort, baseConfig, writeConfig, runCsr, startMockNode, cleanup } from '../a1/lib.mjs';
import { verify, bindingMsg } from '../a1/crypto.mjs';
import { startAttestMonitor } from '../a5/attestmon.mjs';
import { V } from '../a5/vectors.mjs';
import { rpc, csrJson } from '../a3b/devnet.mjs';
import { fundRunnerCell, signAndSubmit } from '../a3b/runner.mjs';
import { CKB, deploy, runner, deposit, createPayout, prepareRegister, payoutArgs } from './flow.mjs';

after(cleanup);

// ---------- register attest (no devnet) ----------

async function attestEnv(name, monUrl) {
  const dir = tmpDir(name);
  const keys = path.join(dir, 'testnet', 'keys');
  fs.mkdirSync(keys, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(keys, 'service.key'), V.service_key + '\n', { mode: 0o600 });
  const node = await startMockNode({ chain: 'ckb_testnet' });
  const cfg = baseConfig({ rpcPort: node.port, dashPort: await freePort(), pingPort: await freePort() });
  cfg.monitor = { url: monUrl, pubkey: V.monitor_pubkey };
  return { args: ['--data-root', dir, '--config', writeConfig(dir, cfg)], out: path.join(dir, 'att', 'attestation.json') };
}

const STAKE = '0x' + 'ab'.repeat(32) + ':2';

test('register attest sends the stake', async () => {
  const mon = await startAttestMonitor(() => [200, V.ok]);
  const e = await attestEnv('a7-att-ok', mon.url);
  const up = '0x' + 'AB'.repeat(32) + ':2';
  const r = await runCsr([...e.args, 'register', 'attest', '--stake', up, '--out', e.out, '--json']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(mon.requests.length, 1);
  const q = mon.requests[0];
  assert.deepEqual(Object.keys(q).sort(), ['binding_sig', 'network', 'peer_id', 'service_pubkey', 'stake']);
  assert.equal(q.stake, STAKE, 'the out point is sent in lower case');
  assert.ok(verify(V.service_pubkey, bindingMsg(V.peer_id, V.service_pubkey, 'testnet'), q.binding_sig));
  assert.deepEqual(JSON.parse(fs.readFileSync(e.out, 'utf8')), V.ok);
});

test('register attest: the stake is required and checked before asking the monitor', async () => {
  const mon = await startAttestMonitor(() => [200, V.ok]);
  const e = await attestEnv('a7-att-bad', mon.url);
  const none = await runCsr([...e.args, 'register', 'attest', '--out', e.out, '--json']);
  assert.notEqual(none.code, 0, '--stake is required');
  for (const bad of ['0x12:0', '0x' + 'ab'.repeat(32), '0x' + 'ab'.repeat(32) + ':x', 'ab'.repeat(32) + ':0']) {
    const r = await runCsr([...e.args, 'register', 'attest', '--stake', bad, '--out', e.out, '--json']);
    assert.notEqual(r.code, 0, bad);
    assert.match(r.stderr, /stake/i);
  }
  assert.equal(mon.requests.length, 0, 'the monitor must not be asked with a malformed stake');
});

test('register attest reports stake errors of the monitor', async () => {
  const mon = await startAttestMonitor(() => [403, { error: 'stake_invalid', reason: 'too_small' }]);
  const e = await attestEnv('a7-att-403', mon.url);
  const r = await runCsr([...e.args, 'register', 'attest', '--stake', STAKE, '--out', e.out, '--json']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /stake_invalid/);
  assert.match(r.stderr, /too_small/);
  assert.equal(fs.existsSync(e.out), false);
});

// ---------- devnet ----------

test('register prepare refuses without payout cell, without stake or with a bad stake', async () => {
  await deploy();
  const r = await runner('a7-reg-refuse', payoutArgs('a7-reg-refuse'));
  await fundRunnerCell(r, 200);
  const good = await deposit(10000);

  const noPayout = await prepareRegister(r, good);
  assert.notEqual(noPayout.code, 0, 'no payout cell yet');
  assert.match(noPayout.stderr, /payout/i);

  await createPayout(r, 100);
  const noStake = await prepareRegister(r, undefined);
  assert.notEqual(noStake.code, 0, '--stake is required');

  const small = await deposit(9999);
  const tooSmall = await prepareRegister(r, small);
  assert.notEqual(tooSmall.code, 0);
  assert.match(tooSmall.stderr, /stake/i);

  const rc = (await csrJson([...r.args, 'runner-cell', 'status'])).json;
  const notDao = await prepareRegister(r, rc.outpoint);
  assert.notEqual(notDao.code, 0);
  assert.match(notDao.stderr, /stake/i);

  const missing = await prepareRegister(r, '0x' + '00'.repeat(32) + ':0');
  assert.notEqual(missing.code, 0);
  assert.match(missing.stderr, /stake/i);
});

test('register with a DAO stake: the stake is the first cell dep, status shows stake and payout', async () => {
  await deploy();
  const r = await runner('a7-reg-ok', payoutArgs('a7-reg-ok'));
  await fundRunnerCell(r, 200);
  await createPayout(r, 100);
  const stake = await deposit(10000);
  const p = await prepareRegister(r, stake);
  assert.equal(p.code, 0, p.stderr);
  const unsigned = JSON.parse(fs.readFileSync(p.file, 'utf8'));
  const tx = unsigned.transaction || unsigned;
  const [sTx, sIdx] = stake.split(':');
  assert.deepEqual(tx.cell_deps[0], { out_point: { tx_hash: sTx, index: '0x' + Number(sIdx).toString(16) }, dep_type: 'code' });
  const h = await signAndSubmit(r, p.file);
  const committed = await rpc('get_transaction', [h]);
  assert.equal(committed.tx_status.status, 'committed');

  const st = (await csrJson([...r.args, 'register', 'status'])).json;
  const pay = (await csrJson([...r.args, 'payout', 'status'])).json;
  assert.equal(st.registered, true);
  assert.equal(st.payout_lock_hash, pay.lock_hash);
  assert.deepEqual(st.stake, { outpoint: stake, status: 'deposited', capacity_shannons: 10000 * CKB });
  assert.ok(st.attestation_expiry_ms > Date.now() - 86400000);
  assert.equal(st.float_lock_hash, undefined);
});

test('register status before registering', async () => {
  await deploy();
  const r = await runner('a7-reg-none', payoutArgs('a7-reg-none'));
  const st = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.deepEqual(st, { registered: false, outpoint: null, peer_id: null, attestation_expiry_ms: null,
    payout_lock_hash: null, stake: null });
});
