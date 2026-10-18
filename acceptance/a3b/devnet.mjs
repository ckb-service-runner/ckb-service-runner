// Helpers for tests against the local devnet started by scripts/devnet.sh.
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { ROOT, runCsr } from '../a1/lib.mjs';

export const RPC = 'http://127.0.0.1:18114';
export const CSR_DEV = path.join(ROOT, 'target', 'debug', 'csr-dev');
export const DEPLOYMENTS = path.join(ROOT, 'deployments', 'devnet.json');

let rpcId = 0;
export async function rpc(method, params = []) {
  rpcId += 1;
  const r = await fetch(RPC, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: rpcId, method, params }),
  });
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error)}`);
  return j.result;
}

export function mine(n = 1) {
  const r = spawnSync('bash', ['scripts/devnet.sh', 'mine', String(n)], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`devnet mine failed: ${r.stderr}`);
}

export async function tipNumber() {
  return Number(BigInt((await rpc('get_tip_header')).number));
}

export async function csrDev(args) {
  const { spawn } = await import('node:child_process');
  return new Promise((resolve) => {
    const p = spawn(CSR_DEV, [...args, '--json'], { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    p.stdout.on('data', (d) => { stdout += d; });
    p.stderr.on('data', (d) => { stderr += d; });
    p.on('error', (e) => resolve({ code: -1, stdout, stderr: String(e), json: null }));
    p.on('close', (code) => {
      let json = null;
      try { json = JSON.parse(stdout); } catch { /* not JSON */ }
      resolve({ code, stdout, stderr, json });
    });
  });
}

export async function csrJson(args) {
  const r = await runCsr([...args, '--json'], { timeoutMs: 180000 });
  let json = null;
  try { json = JSON.parse(r.stdout); } catch { /* not JSON */ }
  return { ...r, json };
}

export async function liveCell(txHash, index) {
  return rpc('get_live_cell', [{ tx_hash: txHash, index: '0x' + Number(index).toString(16) }, true]);
}

export async function waitCommitted(txHash, timeoutMs = 60000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    const t = await rpc('get_transaction', [txHash]);
    if (t && t.tx_status && t.tx_status.status === 'committed') return t;
    if (t && t.tx_status && t.tx_status.status === 'rejected') throw new Error(`tx rejected: ${JSON.stringify(t.tx_status)}`);
    mine(1);
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`tx ${txHash} not committed`);
}
