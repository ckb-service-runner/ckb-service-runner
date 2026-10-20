// A7 Phase 1: contracts – Registry v2 (payout lock hash, DAO stake), float lock removed.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT } from '../a1/lib.mjs';

const read = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');

test('the float lock is gone', () => {
  assert.equal(fs.existsSync(path.join(ROOT, 'contracts', 'csr-float-lock')), false, 'contracts/csr-float-lock must be deleted');
  const ws = read('contracts/Cargo.toml');
  assert.doesNotMatch(ws, /csr-float-lock/);
  assert.doesNotMatch(read('scripts/build-contracts.sh'), /csr-float-lock/);
  for (const name of ['csr-dual-lock', 'csr-registry', 'csr-job-type', 'csr-job-lock']) {
    assert.ok(fs.existsSync(path.join(ROOT, 'contracts', 'build', 'release', name)), `${name} not built`);
  }
  assert.equal(fs.existsSync(path.join(ROOT, 'contracts', 'build', 'release', 'csr-float-lock')), false,
    'a stale csr-float-lock binary must not remain in contracts/build/release');
});

test('schema: Registry v2', () => {
  const mol = read('contracts/schemas/csr.mol');
  const reg = mol.match(/table Registry \{([^}]*)\}/);
  assert.ok(reg, 'table Registry missing');
  const fields = [...reg[1].matchAll(/^\s*(\w+)\s*:\s*(\w+)\s*,/gm)].map((m) => [m[1], m[2]]);
  assert.deepEqual(fields, [
    ['peer_id', 'Bytes'], ['service_pubkey', 'Byte33'], ['payout_lock_hash', 'Byte32'], ['network', 'byte'],
    ['attestation', 'Attestation'], ['prekeys', 'Bytes'], ['stake', 'OutPoint'],
  ]);
  const op = mol.match(/struct OutPoint \{([^}]*)\}/);
  assert.ok(op, 'struct OutPoint missing');
  assert.deepEqual([...op[1].matchAll(/^\s*(\w+)\s*:\s*(\w+)\s*,/gm)].map((m) => [m[1], m[2]]),
    [['tx_hash', 'Byte32'], ['index', 'Uint32']]);
  assert.doesNotMatch(mol, /float/i);
});

test('registry contract: stake constants', () => {
  const src = read('contracts/csr-registry/src/main.rs');
  assert.match(src, /82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e/, 'DAO type code hash');
  assert.match(src, /MIN_STAKE/);
  assert.match(src, /1_000_000_000_000|10_000 \* CKB|10_000u64 \* 100_000_000|1000000000000/, 'MIN_STAKE = 10,000 CKB');
  for (const c of [18, 19, 20, 21]) assert.match(src, new RegExp(`:\\s*i8\\s*=\\s*${c};`), `error code ${c}`);
  assert.doesNotMatch(src, /float/i);
  assert.doesNotMatch(read('contracts/csr-job-lock/src/main.rs'), /float/i);
});

test('spec/registry.md describes v2', () => {
  const s = read('spec/registry.md');
  for (const w of ['payout_lock_hash', 'stake', 'OutPoint', 'MIN_STAKE', '10,000 CKB',
    '0x82d76d1b75fe2fd9a27dfbaa65a039221a380d76c926f378d3f81cf3e7e13f2e', '| 18 |', '| 19 |', '| 20 |', '| 21 |']) {
    assert.ok(s.includes(w), `spec/registry.md must mention ${w}`);
  }
  assert.doesNotMatch(s, /float_lock_hash/);
});
