// Devnet helpers for job tests.
import assert from 'node:assert/strict';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { ROOT, runCsr } from '../a1/lib.mjs';
import { csrDev, waitCommitted, DEPLOYMENTS } from '../a3b/devnet.mjs';

export const ELF = (name) => path.join(ROOT, 'services', 'rust', 'target', 'riscv64imac-unknown-none-elf', 'release', name);

export async function deployContracts() {
  const r = await csrDev(['deploy-contracts', '--out', DEPLOYMENTS]);
  assert.equal(r.code, 0, r.stderr);
}

export async function deployService(name) {
  const r = await csrDev(['deploy-service', '--elf', ELF(name)]);
  assert.equal(r.code, 0, r.stderr);
  await waitCommitted(r.json.tx_hash);
  return r.json;
}

export async function allow(runner, svc) {
  const add = await runCsr([...runner.args, 'service', 'add-onchain', '--outpoint', `${svc.tx_hash}:${svc.index}`], { timeoutMs: 60000 });
  assert.equal(add.code, 0, add.stderr);
  assert.equal(JSON.parse(add.stdout).code_hash, svc.code_hash);
  const ap = await runCsr([...runner.args, 'service', 'approve', svc.code_hash]);
  assert.equal(ap.code, 0, ap.stderr);
}

export async function createJob(codeHash, { tipCkb, maxCycles = 10000000, trigger = 'ondemand', runs = 1, maxFee = 100000 }) {
  const r = await csrDev(['job', '--code-hash', codeHash, '--tip-ckb', String(tipCkb), '--max-fee-shannons', String(maxFee),
    '--max-cycles', String(maxCycles), '--trigger', trigger, '--runs', String(runs)]);
  assert.equal(r.code, 0, r.stderr);
  await waitCommitted(r.json.tx_hash);
  return `${r.json.tx_hash}:${r.json.index}`;
}

// Mines one block every intervalMs until stop() is called.
export function startMiner(intervalMs = 500) {
  let running = true;
  const loop = async () => {
    while (running) {
      await new Promise((resolve) => {
        const p = spawn('bash', ['scripts/devnet.sh', 'mine', '1'], { stdio: 'ignore' });
        p.on('close', resolve);
        p.on('error', resolve);
      });
      await new Promise((r) => setTimeout(r, intervalMs));
    }
  };
  const done = loop();
  return async () => { running = false; await done; };
}
