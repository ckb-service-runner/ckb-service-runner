// Creates a devnet CSR data root and walks it through the operator flows via the CLIs.
import assert from 'node:assert/strict';
import path from 'node:path';
import { tmpDir, baseConfig, writeConfig, freePort, runCsr } from '../a1/lib.mjs';
import { csrDev, csrJson, waitCommitted, DEPLOYMENTS } from './devnet.mjs';

export async function devnetRunner(name, mutate = (c) => c) {
  const dir = tmpDir(name);
  const dashPort = await freePort();
  const pingPort = await freePort();
  const cfg = mutate(Object.assign(baseConfig({ rpcPort: 18114, dashPort, pingPort }), { network: 'devnet', devnet: { deployments: DEPLOYMENTS } }));
  const configPath = writeConfig(dir, cfg);
  const args = ['--data-root', dir, '--config', configPath];
  const id = await csrJson([...args, 'identity']);
  assert.equal(id.code, 0, id.stderr);
  return { dir, cfg, configPath, args, id: id.json, base: `http://127.0.0.1:${dashPort}` };
}

export async function signAndSubmit(r, unsignedFile) {
  const signed = `${unsignedFile}.signed.json`;
  const s = await csrDev(['sign-tx', '--in', unsignedFile, '--out', signed, '--key', 'genesis']);
  assert.equal(s.code, 0, s.stderr);
  const sub = await csrJson([...r.args, 'tx', 'submit', signed]);
  assert.equal(sub.code, 0, sub.stderr);
  await waitCommitted(sub.json.tx_hash);
  return sub.json.tx_hash;
}

export async function fundRunnerCell(r, ckb = 200) {
  const f = await csrDev(['fund', '--lock-args', r.id.runner_lock_args, '--ckb', String(ckb)]);
  assert.equal(f.code, 0, f.stderr);
  await waitCommitted(f.json.tx_hash);
}

export async function createFloat(r, ckb = 1000, maxFee = 1000000) {
  const owner = (await csrDev(['owner-lock-args'])).json;
  const out = path.join(r.dir, 'float-create.json');
  const p = await runCsr([...r.args, 'float', 'prepare-create', '--ckb', String(ckb), '--owner-lock-args', owner.lock_args,
    '--max-fee-shannons', String(maxFee), '--out', out], { timeoutMs: 120000 });
  assert.equal(p.code, 0, p.stderr);
  await signAndSubmit(r, out);
  return owner;
}

export async function register(r) {
  const owner = (await csrDev(['owner-lock-args'])).json;
  const att = path.join(r.dir, 'attestation.json');
  const peer = `QmDevnetRunner${path.basename(r.dir).replace(/[^A-Za-z0-9]/g, '')}`;
  const a = await csrDev(['attest', '--peer-id', peer, '--service-pubkey', r.id.service_pubkey, '--network', 'devnet', '--out', att]);
  assert.equal(a.code, 0, a.stderr);
  const out = path.join(r.dir, 'register.json');
  const p = await runCsr([...r.args, 'register', 'prepare', '--attestation', att, '--owner-lock-args', owner.lock_args, '--out', out], { timeoutMs: 120000 });
  assert.equal(p.code, 0, p.stderr);
  await signAndSubmit(r, out);
}
