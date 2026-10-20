// Devnet helpers for the A7 tests (stake + payout). The devnet is started by scripts/devnet.sh.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import crypto from 'node:crypto';
import path from 'node:path';
import { tmpDir, baseConfig, writeConfig, freePort, runCsr } from '../a1/lib.mjs';
import { csrDev, csrJson, waitCommitted, DEPLOYMENTS } from '../a3b/devnet.mjs';
import { signAndSubmit } from '../a3b/runner.mjs';

export const CKB = 1e8;
export const DAO_TYPE = {
  code_hash: '0x82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e', hash_type: 'type', args: '0x',
};
export const ACP_DEVNET_CODE_HASH = '0xcd69ba816f7471e59110058aa37387c362ed9a240cd178f7bb1ecee386cb31e6';
export const ACP_TESTNET = {
  code_hash: '0x3419a1c09eb2567f6552ee7a8ecffd64155cffe0f1796e6e61ec088d740c1356', hash_type: 'type',
  tx_hash: '0xec26b0f85ed839ece5f11c4c4e837ec359f5adc4420410f6453b1f6b60fb96a6', index: 0, dep_type: 'dep_group',
};

// Distinct 20-byte payout lock args per test runner (anyone-can-pay args need no key unless withdrawing).
export const payoutArgs = (name) => '0x' + crypto.createHash('sha256').update(name).digest('hex').slice(0, 40);

export async function deploy() {
  const r = await csrDev(['deploy-contracts', '--out', DEPLOYMENTS]);
  assert.equal(r.code, 0, r.stderr);
  return JSON.parse(fs.readFileSync(DEPLOYMENTS, 'utf8'));
}

export async function owner() {
  const o = await csrDev(['owner-lock-args']);
  assert.equal(o.code, 0, o.stderr);
  return o.json;          // {lock_args, lock_hash} of the genesis ("operator wallet") key
}

// A devnet CSR data root; payout.lock_args = payoutArgs ('' = not set).
export async function runner(name, payoutArgs, mutate = (c) => c) {
  const dir = tmpDir(name);
  const dashPort = await freePort();
  const pingPort = await freePort();
  const cfg = mutate(Object.assign(baseConfig({ rpcPort: 18114, dashPort, pingPort }),
    { network: 'devnet', devnet: { deployments: DEPLOYMENTS }, payout: { lock_args: payoutArgs } }));
  const configPath = writeConfig(dir, cfg);
  const args = ['--data-root', dir, '--config', configPath];
  const id = await csrJson([...args, 'identity']);
  assert.equal(id.code, 0, id.stderr);
  return { dir, cfg, configPath, args, id: id.json, base: `http://127.0.0.1:${dashPort}` };
}

// A Nervos DAO deposit of the genesis key; returns "0x<tx>:<index>".
export async function deposit(ckb) {
  const d = await csrDev(['dao-deposit', '--ckb', String(ckb)]);
  assert.equal(d.code, 0, d.stderr);
  assert.equal(d.json.index, 0);
  await waitCommitted(d.json.tx_hash);
  return `${d.json.tx_hash}:${d.json.index}`;
}

export async function createPayout(r, ckb) {
  const o = await owner();
  const out = path.join(r.dir, `payout-create-${ckb}.json`);
  const p = await runCsr([...r.args, 'payout', 'prepare-create', '--ckb', String(ckb), '--owner-lock-args', o.lock_args,
    '--out', out], { timeoutMs: 120000 });
  assert.equal(p.code, 0, p.stderr);
  return signAndSubmit(r, out);
}

export async function attestFile(r, peer) {
  const att = path.join(r.dir, `att-${peer}.json`);
  const a = await csrDev(['attest', '--peer-id', peer, '--service-pubkey', r.id.service_pubkey, '--network', 'devnet', '--out', att]);
  assert.equal(a.code, 0, a.stderr);
  return att;
}

export async function prepareRegister(r, stake, peer = `QmA7${path.basename(r.dir).replace(/[^A-Za-z0-9]/g, '')}`) {
  const o = await owner();
  const att = await attestFile(r, peer);
  const out = path.join(r.dir, `register-${Date.now()}.json`);
  const args = [...r.args, 'register', 'prepare', '--attestation', att, '--owner-lock-args', o.lock_args, '--out', out];
  if (stake !== undefined) args.push('--stake', stake);
  const p = await runCsr(args, { timeoutMs: 120000 });
  return { ...p, file: out };
}

export async function register(r, stake) {
  const p = await prepareRegister(r, stake);
  assert.equal(p.code, 0, p.stderr);
  return signAndSubmit(r, p.file);
}
