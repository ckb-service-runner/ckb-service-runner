import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { runCsr, cleanup } from '../a1/lib.mjs';
import { csrDev, csrJson } from './devnet.mjs';
import { devnetRunner, fundRunnerCell, createFloat, register } from './runner.mjs';

after(cleanup);

test('registration needs a float and a matching attestation', async () => {
  const r = await devnetRunner('a3b-reg');
  const owner = (await csrDev(['owner-lock-args'])).json;
  const att = path.join(r.dir, 'att.json');
  assert.equal((await csrDev(['attest', '--peer-id', 'QmX', '--service-pubkey', r.id.service_pubkey, '--network', 'devnet', '--out', att])).code, 0);
  const noFloat = await runCsr([...r.args, 'register', 'prepare', '--attestation', att, '--owner-lock-args', owner.lock_args, '--out', path.join(r.dir, 'x.json')]);
  assert.notEqual(noFloat.code, 0, 'registration without a float must be refused');

  await fundRunnerCell(r, 200);
  await createFloat(r, 1000);
  const other = await devnetRunner('a3b-other');
  const wrong = path.join(r.dir, 'wrong.json');
  assert.equal((await csrDev(['attest', '--peer-id', 'QmX', '--service-pubkey', other.id.service_pubkey, '--network', 'devnet', '--out', wrong])).code, 0);
  const mismatch = await runCsr([...r.args, 'register', 'prepare', '--attestation', wrong, '--owner-lock-args', owner.lock_args, '--out', path.join(r.dir, 'y.json')]);
  assert.notEqual(mismatch.code, 0, 'an attestation for another key must be refused');

  await register(r);
  const st = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.equal(st.registered, true);
  const fl = (await csrJson([...r.args, 'float', 'status'])).json;
  assert.ok(st.float_lock_hash && st.float_lock_hash.startsWith('0x'));
  assert.ok(st.attestation_expiry_ms > Date.now() - 86400000);
  assert.ok(fl.found);
});

test('docs', () => {
  for (const f of ['docs/testing.md', 'docs/milestones/A3b.md']) assert.ok(fs.existsSync(f), `${f} missing`);
  assert.match(fs.readFileSync('docs/testing.md', 'utf8'), /testnet/i);
});
