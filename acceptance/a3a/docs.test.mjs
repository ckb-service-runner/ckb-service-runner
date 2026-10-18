import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const read = (f) => fs.readFileSync(path.join(process.cwd(), f), 'utf8');

test('standards are written', () => {
  const job = read('spec/job-cell-standard.md');
  for (const s of ['OnDemand', 'AtBlock', 'AtEpoch', 'AtTimestamp', 'Every', 'OnCellConsumed', 'tip_shannons', 'max_fee_shannons', 'remaining_runs', 'refund_lock_hash', 'expiry_block', 'since']) {
    assert.ok(job.includes(s), `job-cell-standard.md must cover ${s}`);
  }
  const reg = read('spec/registry.md');
  for (const s of ['CSR-ATTEST-v1', 'config_type_hash', 'clock_ms', 'expiry_ms', 'float_lock_hash', 'prekeys', 'runner']) {
    assert.ok(reg.includes(s), `registry.md must cover ${s}`);
  }
  const fl = read('spec/float-lock.md');
  for (const s of ['owner_lock_hash', 'runner_lock_hash', 'max_fee_shannons']) assert.ok(fl.includes(s), `float-lock.md must cover ${s}`);
  for (const d of ['contracts/schemas', 'contracts/csr-contract-tests']) assert.ok(fs.existsSync(d), `${d} missing`);
});
