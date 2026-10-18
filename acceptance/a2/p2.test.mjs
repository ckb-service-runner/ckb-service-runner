import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { setup, cleanup } from '../a1/lib.mjs';
import { pubFromPrivHex, pubFromCompressed, isLowS } from '../a1/crypto.mjs';
import { runService, asciiHex } from './svc.mjs';

after(cleanup);

const verifyBytes = (pubHex, bytes, sigHex) => crypto.verify(
  'sha256', bytes, { key: pubFromCompressed(pubHex), dsaEncoding: 'ieee-p1363' }, Buffer.from(sigHex.slice(2), 'hex'),
);

function walk(d) {
  if (!fs.existsSync(d)) return [];
  return fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));
}

test('kvtest counter persists across runs; storage is encrypted', async () => {
  const env = await setup('a2-kv');
  const a = await runService(env, 'kvtest');
  assert.equal(a.status, 'ok', JSON.stringify(a));
  assert.equal(a.result_hex, '0x01000000');
  const b = await runService(env, 'kvtest');
  assert.equal(b.result_hex, '0x02000000');
  const dir = path.join(env.dir, 'testnet', 'service_data', a.code_hash.slice(2));
  const files = walk(dir);
  assert.ok(files.length > 0, `no storage files under ${dir}`);
  for (const f of files) {
    assert.ok(!f.includes('counter'), `file name reveals a key: ${f}`);
    assert.ok(!fs.readFileSync(f).includes(Buffer.from('counter')), `plaintext key in ${f}`);
  }
});

test('quota is enforced', async () => {
  const env = await setup('a2-quota', { cfg: (c) => { c.storage.per_service_quota_mb = 1; return c; } });
  const r = await runService(env, 'quotatest');
  assert.equal(r.status, 'ok', JSON.stringify(r));
  assert.equal(r.result_hex, asciiHex('quota-error'));
});

test('sign is domain-separated and verifiable', async () => {
  const env = await setup('a2-sign');
  const msg = crypto.randomBytes(32);
  const r = await runService(env, 'signtest', '0x' + msg.toString('hex'));
  assert.equal(r.status, 'ok', JSON.stringify(r));
  const sig = r.result_hex;
  assert.equal(sig.length, 2 + 128);
  assert.ok(isLowS(sig));
  const pub = pubFromPrivHex(fs.readFileSync(path.join(env.dir, 'testnet', 'keys', 'service.key'), 'utf8'));
  const signed = Buffer.concat([Buffer.from('CSR-SERVICE-SIGN-v1\n', 'ascii'), Buffer.from(r.code_hash.slice(2), 'hex'), msg]);
  assert.ok(verifyBytes(pub, signed, sig), 'signature does not verify');
  assert.ok(!verifyBytes(pub, msg, sig), 'the raw message must not be signed directly');
});
