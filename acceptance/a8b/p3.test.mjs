// A8b (devnet): as acceptance/a8/p3, but the operator sends the documented 700 CKB (no devnet faucet).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { runCsr, cleanup } from '../a1/lib.mjs';
import { rpc, csrDev, csrJson, waitCommitted, liveCell } from '../a3b/devnet.mjs';
import { deploy, owner, deposit, attestFile } from '../a7/flow.mjs';
import { CKB, runner, fundRunnerCell, signMessage, peerId, freshAddress, lockCapacity, wallet, registerWithConsent } from '../a8/flow.mjs';
import { consentText, secpAddress } from '../a8/addr.mjs';

after(cleanup);

async function inputLockArgs(t) {
  const out = [];
  for (const i of t.inputs) {
    const prev = await rpc('get_transaction', [i.previous_output.tx_hash]);
    out.push(prev.transaction.outputs[Number(i.previous_output.index)].lock.args);
  }
  return out;
}

test('sign-message and verify-consent agree', async () => {
  await deploy();
  const o = await owner();
  const s = await signMessage('Message from CSR');
  assert.equal(s.lock_args, o.lock_args);
  const r = await runner('a8-verify-dev');
  const v = await csrJson([...r.args, 'register', 'verify-consent', '--message', 'Message from CSR', '--signature', s.signature]);
  assert.equal(v.code, 0, v.stderr);
  assert.equal(v.json.lock_args, o.lock_args);
});

test('registration paid and owned by the runner wallet, consent of the stake owner in the witness', async () => {
  await deploy();
  const o = await owner();
  const r = await runner('a8-reg');
  await fundRunnerCell(r, 700);
  const stake = await deposit(10000);

  const m = await csrJson([...r.args, 'register', 'consent-message', '--stake', stake.replace(/[a-f]/g, (c) => c.toUpperCase()).replace('0X', '0x')]);
  assert.equal(m.code, 0, m.stderr);
  const text = consentText({ network: 'devnet', stake, servicePubkey: r.id.service_pubkey, peerId: await peerId() });
  assert.deepEqual(m.json, { message: text, stake_owner_lock_args: o.lock_args, stake_owner_address: secpAddress(o.lock_args, 'devnet') });

  const att = await attestFile(r, await peerId());
  const create = (sig) => runCsr([...r.args, 'register', 'create', '--stake', stake, '--consent', sig, '--attestation', att, '--json'],
    { timeoutMs: 180000 });
  // consent of another key, or over another text: refused before anything is sent
  const other = await signMessage(text, '0x' + '42'.repeat(32));
  const refused = await create(other.signature);
  assert.notEqual(refused.code, 0);
  assert.match(refused.stderr, /consent/i);
  const wrongText = await signMessage(text + 'x');
  const refused2 = await create(wrongText.signature);
  assert.notEqual(refused2.code, 0);
  assert.match(refused2.stderr, /consent/i);

  const walletBefore = (await wallet(r)).capacity_shannons;
  const good = await signMessage(text);
  const reg = await create(good.signature);
  assert.equal(reg.code, 0, reg.stderr);
  const j = JSON.parse(reg.stdout.trim());
  assert.deepEqual(Object.keys(j).sort(), ['outpoint', 'stake_owner_lock_args', 'tx_hash', 'updated']);
  assert.equal(j.updated, false);
  assert.equal(j.stake_owner_lock_args, o.lock_args);
  const t = (await waitCommitted(j.tx_hash)).transaction;

  for (const a of await inputLockArgs(t)) assert.equal(a, r.id.runner_lock_args, 'only runner wallet cells may be spent');
  const [sTx, sIdx] = stake.split(':');
  assert.deepEqual(t.cell_deps[0], { out_point: { tx_hash: sTx, index: '0x' + Number(sIdx).toString(16) }, dep_type: 'code' });
  const regIdx = t.outputs.findIndex((x) => x.type);
  assert.ok(regIdx >= 0, 'registry output');
  const rh = r.id.runner_lock_hash.slice(2);
  assert.equal(t.outputs[regIdx].lock.args, '0x' + rh + rh, 'dual lock: owner = runner = the runner wallet lock');
  assert.ok(t.witnesses[regIdx].includes(good.signature.slice(2)), 'consent signature in the witness of the registry output');
  assert.equal(j.outpoint, `${j.tx_hash}:${regIdx}`);

  const st = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.equal(st.registered, true);
  assert.equal(st.outpoint, j.outpoint);
  assert.equal(st.payout_lock_hash, r.id.runner_lock_hash, 'tips go to the runner wallet');
  assert.deepEqual(st.stake, { outpoint: stake, status: 'deposited', capacity_shannons: 10000 * CKB });
  const regCap = Number(BigInt(t.outputs[regIdx].capacity));
  const walletAfter = (await wallet(r)).capacity_shannons;
  assert.ok(walletAfter < walletBefore - regCap && walletAfter > walletBefore - regCap - CKB,
    `the runner wallet pays the registry cell and the fee (${walletBefore} -> ${walletAfter}, cell ${regCap})`);

  // a new deposit: the same registry cell is updated (new stake, new consent)
  const stake2 = await deposit(10000);
  const upd = await registerWithConsent(r, stake2);
  assert.equal(upd.updated, true);
  assert.notEqual(upd.outpoint, j.outpoint);
  assert.equal((await liveCell(...j.outpoint.split(':').map((v, k) => (k ? Number(v) : v)))).status === 'live', false);
  const st2 = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.equal(st2.outpoint, upd.outpoint);
  assert.equal(st2.stake.outpoint, stake2);

  // deregister: the registry cell's capacity returns to the runner wallet
  const w1 = (await wallet(r)).capacity_shannons;
  const rm = await csrJson([...r.args, 'register', 'remove']);
  assert.equal(rm.code, 0, rm.stderr);
  await waitCommitted(rm.json.tx_hash);
  assert.equal(typeof rm.json.capacity_shannons, 'number');
  assert.equal((await csrJson([...r.args, 'register', 'status'])).json.registered, false);
  const w2 = (await wallet(r)).capacity_shannons;
  assert.ok(w2 > w1 + regCap - CKB, `the registry capacity returns to the wallet (${w1} -> ${w2})`);
});

test('register create explains a runner wallet that is too small', async () => {
  await deploy();
  const r = await runner('a8-poor');
  await fundRunnerCell(r, 150);
  const stake = await deposit(10000);
  const text = consentText({ network: 'devnet', stake, servicePubkey: r.id.service_pubkey, peerId: await peerId() });
  const good = await signMessage(text);
  const att = await attestFile(r, await peerId());
  const reg = await runCsr([...r.args, 'register', 'create', '--stake', stake, '--consent', good.signature, '--attestation', att, '--json'],
    { timeoutMs: 120000 });
  assert.notEqual(reg.code, 0);
  assert.match(reg.stderr, /runner wallet/i);
  assert.ok(reg.stderr.includes(r.id.runner_address), 'the error names the runner address to send CKB to');
});

test('wallet status, send and sweep', async () => {
  await deploy();
  const target = freshAddress('a8-sweep');
  const r = await runner('a8-wallet', { sweep_address: target.address, sweep_threshold_ckb: 300, sweep_amount_ckb: 200 });
  await fundRunnerCell(r, 250);

  const w = await wallet(r);
  // Integration (Oct 2026): csr-runner-fixes added sweep_warning to the wallet status.
  assert.deepEqual(Object.keys(w).sort(), ['address', 'capacity_shannons', 'last_sweep', 'sweep_address', 'sweep_amount_ckb', 'sweep_threshold_ckb', 'sweep_warning']);
  assert.equal(w.address, r.id.runner_address);
  assert.equal(w.capacity_shannons, 250 * CKB);
  assert.equal(w.sweep_address, target.address);
  assert.equal(w.sweep_threshold_ckb, 300);
  assert.equal(w.sweep_amount_ckb, 200);
  assert.equal(w.last_sweep, null);

  // below the threshold nothing happens
  const s0 = await csrJson([...r.args, 'wallet', 'sweep']);
  assert.equal(s0.code, 0, s0.stderr);
  assert.deepEqual(s0.json, { swept: false, tx_hash: null, amount_shannons: null });

  // at the threshold: exactly sweep_amount_ckb goes to the sweep address
  await fundRunnerCell(r, 100);
  const s1 = await csrJson([...r.args, 'wallet', 'sweep']);
  assert.equal(s1.code, 0, s1.stderr);
  assert.equal(s1.json.swept, true);
  assert.equal(s1.json.amount_shannons, 200 * CKB);
  await waitCommitted(s1.json.tx_hash);
  assert.equal(await lockCapacity(target.args), 200 * CKB);
  const after1 = await wallet(r);
  assert.ok(after1.capacity_shannons < 150 * CKB && after1.capacity_shannons > 149 * CKB, String(after1.capacity_shannons));
  assert.equal(after1.last_sweep.tx_hash, s1.json.tx_hash);
  assert.equal(after1.last_sweep.amount_shannons, 200 * CKB);

  // manual send; refused when less than 61 CKB would stay in the wallet
  const dest = freshAddress('a8-send');
  const tooMuch = await csrJson([...r.args, 'wallet', 'send', '--to', dest.address, '--ckb', '100']);
  assert.notEqual(tooMuch.code, 0);
  assert.match(tooMuch.stderr, /runner wallet/i);
  const sent = await csrJson([...r.args, 'wallet', 'send', '--to', dest.address, '--ckb', '70']);
  assert.equal(sent.code, 0, sent.stderr);
  await waitCommitted(sent.json.tx_hash);
  assert.equal(await lockCapacity(dest.args), 70 * CKB);
  const badAddr = await csrJson([...r.args, 'wallet', 'send', '--to', secpAddress(dest.args, 'mainnet'), '--ckb', '61']);
  assert.notEqual(badAddr.code, 0, 'a mainnet address on the devnet is refused');

  // without a sweep address the sweep never sends
  const r2 = await runner('a8-wallet-nosweep');
  await fundRunnerCell(r2, 2000);
  const s2 = await csrJson([...r2.args, 'wallet', 'sweep']);
  assert.equal(s2.code, 0, s2.stderr);
  assert.deepEqual(s2.json, { swept: false, tx_hash: null, amount_shannons: null });
});

test('csr-dev reclaim frees a plain data cell and refuses typed cells', async () => {
  const dep = await deploy();
  const code = dep.scripts.job_type;
  const cfg = dep.config_cell;
  const typed = await csrDev(['reclaim', '--outpoint', `${cfg.tx_hash}:${cfg.index}`]);
  assert.notEqual(typed.code, 0, 'cells with a type script must not be reclaimed');
  const rc = await csrDev(['reclaim', '--outpoint', `${code.tx_hash}:${code.index}`]);
  assert.equal(rc.code, 0, rc.stderr);
  await waitCommitted(rc.json.tx_hash);
  assert.notEqual((await liveCell(code.tx_hash, code.index)).status, 'live');
  assert.equal(typeof rc.json.capacity_shannons, 'number');
});
