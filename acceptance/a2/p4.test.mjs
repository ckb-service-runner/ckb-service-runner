import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { setup, cleanup } from '../a1/lib.mjs';
import { runService, asciiHex } from './svc.mjs';

after(cleanup);

test('Rust, C and JS hello give identical results', async () => {
  const env = await setup('a2-langs');
  for (const payload of ['0x01020304', '0x', '0x' + 'ab'.repeat(100)]) {
    const results = {};
    for (const name of ['hello-rust', 'hello-c', 'hello-js']) {
      const r = await runService(env, name, payload);
      assert.equal(r.status, 'ok', `${name}: ${JSON.stringify(r)}`);
      results[name] = r.result_hex;
    }
    const expected = asciiHex('hello:') + payload.slice(2);
    assert.deepEqual(results, { 'hello-rust': expected, 'hello-c': expected, 'hello-js': expected });
  }
});

test('the three hellos have different code hashes', async () => {
  const env = await setup('a2-hashes');
  const hashes = new Set();
  for (const name of ['hello-rust', 'hello-c', 'hello-js']) hashes.add((await runService(env, name, '0x')).code_hash);
  assert.equal(hashes.size, 3);
});
