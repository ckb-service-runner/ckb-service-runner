// 0.6.1: random_bytes (7050), run through `csr service run-local` with the randtest service.
// Needs the services built (`bash scripts/build-services.sh`).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { setup, cleanup } from '../a1/lib.mjs';
import { runService } from '../a2/svc.mjs';

after(cleanup);

test('randtest_two_draws_differ', async () => {
  const env = await setup('v061-rand');
  const r = await runService(env, 'randtest');
  assert.equal(r.status, 'ok', JSON.stringify(r));
  const out = Buffer.from(r.result_hex.slice(2), 'hex');
  assert.equal(out.length, 68, 'b1 (32) || b2 (32) || four return codes');
  const b1 = out.subarray(0, 32);
  const b2 = out.subarray(32, 64);
  assert.ok(!b1.equals(b2), 'the two draws differ');
  assert.ok(b1.some((x) => x !== 0) && b2.some((x) => x !== 0), 'no draw is all zeros');
  assert.deepEqual([...out.subarray(64)], [0, 0, 11, 12]);
  const debug = JSON.stringify(r.debug ?? []);
  for (const b of [b1, b2]) assert.ok(!debug.includes(b.toString('hex')), 'random bytes in the debug capture');
});
