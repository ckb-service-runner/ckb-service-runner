// Devnet helpers for the A8 tests: runner wallet, consent signatures, runner-owned registration.
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { runCsr } from '../a1/lib.mjs';
import { rpc, csrDev, csrJson, waitCommitted } from '../a3b/devnet.mjs';
import { devnetRunner, fundRunnerCell } from '../a3b/runner.mjs';
import { attestFile } from '../a7/flow.mjs';
import { consentText, secpAddress, SIGHASH_CODE_HASH } from './addr.mjs';

export { fundRunnerCell };
export const CKB = 1e8;

// A devnet runner; `payout` = the [payout] table (sweep settings) or undefined.
export function runner(name, payout, mutate = (c) => c) {
  return devnetRunner(name, (c) => {
    if (payout) c.payout = payout;
    return mutate(c);
  });
}

export async function signMessage(text, key = 'genesis') {
  const s = await csrDev(['sign-message', '--text', text, '--key', key]);
  assert.equal(s.code, 0, s.stderr);
  assert.match(s.json.signature, /^0x[0-9a-f]{130}$/);
  return s.json;
}

export async function peerId() {
  return (await rpc('local_node_info')).node_id;
}

// A fresh address nobody has a key for (only receives).
export function freshAddress(tag) {
  const args = '0x' + crypto.createHash('sha256').update(`${tag}-${Date.now()}-${Math.random()}`).digest('hex').slice(0, 40);
  return { args, address: secpAddress(args, 'devnet') };
}

export async function lockCapacity(lockArgs) {
  const r = await rpc('get_cells_capacity', [{
    script: { code_hash: '0x' + SIGHASH_CODE_HASH, hash_type: 'type', args: lockArgs }, script_type: 'lock',
  }]);
  return r ? Number(BigInt(r.capacity)) : 0;
}

export async function wallet(r) {
  const w = await csrJson([...r.args, 'wallet', 'status']);
  assert.equal(w.code, 0, w.stderr);
  return w.json;
}

// consent-message → sign with the genesis key (the devnet "operator wallet") → register create.
export async function registerWithConsent(r, stake) {
  const m = await csrJson([...r.args, 'register', 'consent-message', '--stake', stake]);
  assert.equal(m.code, 0, m.stderr);
  const peer = await peerId();
  assert.equal(m.json.message, consentText({ network: 'devnet', stake, servicePubkey: r.id.service_pubkey, peerId: peer }));
  const sig = await signMessage(m.json.message);
  const att = await attestFile(r, peer);
  const reg = await runCsr([...r.args, 'register', 'create', '--stake', stake, '--consent', sig.signature, '--attestation', att, '--json'],
    { timeoutMs: 180000 });
  assert.equal(reg.code, 0, reg.stderr);
  const j = JSON.parse(reg.stdout.trim());
  await waitCommitted(j.tx_hash);
  return { ...j, signature: sig.signature, message: m.json.message };
}
