import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { setup, runCsr, cleanup } from '../a1/lib.mjs';
import { runService, asciiHex } from './svc.mjs';

after(cleanup);

function walk(d) {
  if (!fs.existsSync(d)) return [];
  return fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));
}

test('entry is deleted when its outpoint is consumed, and only then', async () => {
  const env = await setup('a2-doc');
  const txHash = crypto.randomBytes(32).toString('hex');
  const index = 3;
  const idx = Buffer.alloc(4);
  idx.writeUInt32LE(index);
  const outpointBytes = '0x01' + txHash + idx.toString('hex');
  const stored = await runService(env, 'dottest', outpointBytes);
  assert.equal(stored.status, 'ok', JSON.stringify(stored));
  assert.equal(stored.result_hex, asciiHex('stored'));
  assert.equal((await runService(env, 'dottest', '0x02')).result_hex, asciiHex('present'));

  for (const f of walk(path.join(env.dir, 'testnet', 'service_data'))) {
    assert.ok(!fs.readFileSync(f).includes(Buffer.from('secret-value')), `plaintext value in ${f}`);
  }

  const other = await runCsr([...env.args, 'storage', 'simulate-consumed', '--outpoint', `0x${txHash}:${index + 1}`]);
  assert.equal(other.code, 0, other.stderr);
  assert.equal(JSON.parse(other.stdout).deleted, 0);
  assert.equal((await runService(env, 'dottest', '0x02')).result_hex, asciiHex('present'));

  const hit = await runCsr([...env.args, 'storage', 'simulate-consumed', '--outpoint', `0x${txHash}:${index}`]);
  assert.equal(hit.code, 0, hit.stderr);
  assert.equal(JSON.parse(hit.stdout).deleted, 1);
  assert.equal((await runService(env, 'dottest', '0x02')).result_hex, asciiHex('absent'));
});
