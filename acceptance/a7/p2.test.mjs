// A7 Phase 2: deployments without the float lock, the vendored anyone-can-pay lock, csr-dev dao-deposit,
// `csr payout …` (testnet search via a chain mock; full flow on the devnet).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { ROOT, tmpDir, freePort, baseConfig, writeConfig, runCsr, cleanup } from '../a1/lib.mjs';
import { startChainMock } from '../a5/chainmock.mjs';
import { csrJson, liveCell } from '../a3b/devnet.mjs';
import { signAndSubmit } from '../a3b/runner.mjs';
import { fileHash, scriptHash } from './ckb.mjs';
import { CKB, DAO_TYPE, ACP_DEVNET_CODE_HASH, ACP_TESTNET, deploy, owner, runner, deposit, createPayout } from './flow.mjs';

after(cleanup);

const ACP_FILE = path.join(ROOT, 'devtools', 'vendor', 'anyone_can_pay');
const ACP_SHA256 = '5eca558b873dcf54e589fa2135a7a0a13defcfd87469cac7dfc6a38cbf57720a';

test('the anyone-can-pay binary is vendored and pinned', () => {
  assert.ok(fs.existsSync(ACP_FILE), 'devtools/vendor/anyone_can_pay missing');
  const sha = crypto.createHash('sha256').update(fs.readFileSync(ACP_FILE)).digest('hex');
  assert.equal(sha, ACP_SHA256);
  assert.equal(fileHash(ACP_FILE), ACP_DEVNET_CODE_HASH);
  const sums = fs.readFileSync(path.join(ROOT, 'devtools', 'vendor', 'SHA256SUMS'), 'utf8');
  assert.match(sums, new RegExp(`^${ACP_SHA256}\\s+\\*?anyone_can_pay$`, 'm'));
});

test('deployments/testnet.json: no float lock, the public anyone-can-pay deployment', () => {
  const dep = JSON.parse(fs.readFileSync(path.join(ROOT, 'deployments', 'testnet.json'), 'utf8'));
  assert.equal(dep.scripts.float_lock, undefined);
  assert.deepEqual(dep.scripts.anyone_can_pay, ACP_TESTNET);
  for (const s of ['dual_lock', 'registry', 'job_type', 'job_lock']) assert.ok(dep.scripts[s], s);
});

async function testnetEnv(name, chain, payoutArgs) {
  const dir = tmpDir(name);
  const cfg = baseConfig({ rpcPort: chain.port, dashPort: await freePort(), pingPort: await freePort() });
  cfg.payout = { lock_args: payoutArgs };
  return ['--data-root', dir, '--config', writeConfig(dir, cfg)];
}

test('payout status on testnet searches the anyone-can-pay lock of payout.lock_args', async () => {
  const chain = await startChainMock({ chain: 'ckb_testnet' });
  const args = '0x' + '11'.repeat(20);
  const r = await runCsr([...(await testnetEnv('a7-payout-tn', chain, args)), 'payout', 'status', '--json']);
  assert.equal(r.code, 0, r.stderr);
  const lock = { code_hash: ACP_TESTNET.code_hash, hash_type: 'type', args };
  assert.deepEqual(JSON.parse(r.stdout.trim()), {
    found: false, outpoint: null, capacity_shannons: null, lock_args: args, lock_hash: scriptHash(lock),
  });
  const s = chain.searches.find((k) => k.script_type === 'lock' && k.script.code_hash === ACP_TESTNET.code_hash);
  assert.ok(s, `no anyone-can-pay search in ${JSON.stringify(chain.searches)}`);
  assert.deepEqual(s.script, lock);
});

test('payout commands need payout.lock_args; the float commands are gone', async () => {
  const chain = await startChainMock({ chain: 'ckb_testnet' });
  const env = await testnetEnv('a7-payout-none', chain, '');
  const r = await runCsr([...env, 'payout', 'status', '--json']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /payout\.lock_args/);
  const f = await runCsr([...env, 'float', 'status', '--json']);
  assert.notEqual(f.code, 0, 'the float subcommand must be removed');
  const bad = await runCsr([...(await testnetEnv('a7-payout-bad', chain, '0x1234')), 'config', 'validate']);
  assert.notEqual(bad.code, 0, 'payout.lock_args must be 20 bytes of hex');
  assert.match(bad.stderr, /payout\.lock_args/);
});

// ---------- devnet ----------

test('deploy-contracts: four CSR contracts plus anyone-can-pay', async () => {
  const dep = await deploy();
  assert.deepEqual(Object.keys(dep.scripts).sort(), ['anyone_can_pay', 'dual_lock', 'job_lock', 'job_type', 'registry']);
  const acp = dep.scripts.anyone_can_pay;
  assert.equal(acp.code_hash, ACP_DEVNET_CODE_HASH);
  assert.equal(acp.hash_type, 'data1');
  assert.equal(acp.dep_type, 'code');
  const cell = await liveCell(acp.tx_hash, acp.index);
  assert.equal(cell.status, 'live');
  assert.equal(cell.cell.data.hash, ACP_DEVNET_CODE_HASH);
});

test('dao-deposit creates a Nervos DAO deposit of the genesis key', async () => {
  await deploy();
  const o = await owner();
  const op = await deposit(10000);
  const [tx, idx] = op.split(':');
  const c = await liveCell(tx, Number(idx));
  assert.equal(c.status, 'live');
  assert.deepEqual(c.cell.output.type, DAO_TYPE);
  assert.equal(c.cell.data.content, '0x0000000000000000');
  assert.equal(Number(BigInt(c.cell.output.capacity)), 10000 * CKB);
  assert.equal(c.cell.output.lock.args, o.lock_args);
  assert.equal(scriptHash(c.cell.output.lock), o.lock_hash);
});

test('payout cell: create, refuse a second one, withdraw part, withdraw all', async () => {
  const dep = await deploy();
  const o = await owner();
  const r = await runner('a7-payout', o.lock_args);
  let st = (await csrJson([...r.args, 'payout', 'status'])).json;
  assert.equal(st.found, false);
  assert.equal(st.lock_args, o.lock_args);
  const lock = { code_hash: ACP_DEVNET_CODE_HASH, hash_type: 'data1', args: o.lock_args };
  assert.equal(st.lock_hash, scriptHash(lock));

  const small = await runCsr([...r.args, 'payout', 'prepare-create', '--ckb', '60', '--owner-lock-args', o.lock_args,
    '--out', path.join(r.dir, 'small.json')], { timeoutMs: 120000 });
  assert.notEqual(small.code, 0, 'a payout cell needs at least 61 CKB');

  await createPayout(r, 100);
  st = (await csrJson([...r.args, 'payout', 'status'])).json;
  assert.equal(st.found, true);
  assert.equal(st.capacity_shannons, 100 * CKB);
  const [tx, idx] = st.outpoint.split(':');
  const c = await liveCell(tx, Number(idx));
  assert.deepEqual(c.cell.output.lock, lock);
  assert.equal(c.cell.output.type, null);
  assert.equal(c.cell.data.content, '0x');
  assert.equal(dep.scripts.anyone_can_pay.code_hash, lock.code_hash);

  const again = await runCsr([...r.args, 'payout', 'prepare-create', '--ckb', '100', '--owner-lock-args', o.lock_args,
    '--out', path.join(r.dir, 'again.json')], { timeoutMs: 120000 });
  assert.notEqual(again.code, 0, 'a second payout cell must be refused');

  const tooMuch = await runCsr([...r.args, 'payout', 'prepare-withdraw', '--ckb', '50', '--out', path.join(r.dir, 'w0.json')],
    { timeoutMs: 120000 });
  assert.notEqual(tooMuch.code, 0, 'the payout cell must keep 61 CKB unless --all');

  const w1 = path.join(r.dir, 'w1.json');
  const p1 = await runCsr([...r.args, 'payout', 'prepare-withdraw', '--ckb', '30', '--out', w1], { timeoutMs: 120000 });
  assert.equal(p1.code, 0, p1.stderr);
  await signAndSubmit(r, w1);
  st = (await csrJson([...r.args, 'payout', 'status'])).json;
  assert.equal(st.capacity_shannons, 70 * CKB);

  const w2 = path.join(r.dir, 'w2.json');
  const p2 = await runCsr([...r.args, 'payout', 'prepare-withdraw', '--all', '--out', w2], { timeoutMs: 120000 });
  assert.equal(p2.code, 0, p2.stderr);
  await signAndSubmit(r, w2);
  st = (await csrJson([...r.args, 'payout', 'status'])).json;
  assert.equal(st.found, false);
});
