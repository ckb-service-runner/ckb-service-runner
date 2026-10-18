// Helpers for running services locally through `csr service run-local`.
import assert from 'node:assert/strict';
import path from 'node:path';
import { ROOT, runCsr } from '../a1/lib.mjs';

export const manifest = (name) => path.join(ROOT, 'services', 'manifests', `${name}.toml`);
export const asciiHex = (s) => '0x' + Buffer.from(s, 'ascii').toString('hex');

export async function runService(env, nameOrPath, payload = '0x', extra = []) {
  const m = nameOrPath.includes('/') ? nameOrPath : manifest(nameOrPath);
  const started = Date.now();
  const r = await runCsr([...env.args, 'service', 'run-local', '--manifest', m, '--payload', payload, ...extra], { timeoutMs: 120000 });
  assert.equal(r.code, 0, `run-local failed (${r.code}):\n${r.stderr}`);
  let j = null;
  try { j = JSON.parse(r.stdout); } catch { /* asserted below */ }
  assert.ok(j, `run-local must print one JSON object, got:\n${r.stdout}`);
  j.elapsedMs = Date.now() - started;
  return j;
}
