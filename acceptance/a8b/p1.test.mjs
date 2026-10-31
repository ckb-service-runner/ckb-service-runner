// A8b Phase 1: the devnet faucet in `csr-dev attest` is gone; the docs ask for the real cost (about 700 CKB).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, cleanup } from '../a1/lib.mjs';
import { rpc, csrDev } from '../a3b/devnet.mjs';
import { deploy, deposit } from '../a7/flow.mjs';
import { runner, lockCapacity } from '../a8/flow.mjs';

after(cleanup);

const read = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');
function walk(d) {
  return fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));
}

test('csr-dev has no faucet any more', () => {
  for (const f of walk(path.join(ROOT, 'devtools', 'csr-dev', 'src'))) {
    assert.doesNotMatch(fs.readFileSync(f, 'utf8'), /faucet/i, `${path.relative(ROOT, f)} still mentions a faucet`);
  }
  assert.match(read('docs/decisions.md'), /A8b/);
});

test('csr-dev attest sends nothing to the runner wallet', async () => {
  await deploy();
  const r = await runner('a8b-attest');
  const peer = (await rpc('local_node_info')).node_id;
  const out = path.join(r.dir, 'att.json');
  const stake = await deposit(10000); // contracts v3: the attestation names the stake
  const a = await csrDev(['attest', '--peer-id', peer, '--stake', stake, '--service-pubkey', r.id.service_pubkey, '--network', 'devnet', '--out', out]);
  assert.equal(a.code, 0, a.stderr);
  assert.equal(a.json.faucet, undefined, 'attest must not report a faucet');
  assert.ok(fs.existsSync(out));
  assert.equal(await lockCapacity(r.id.runner_lock_args), 0, 'the runner wallet must stay empty');
  const bad = await csrDev(['attest', '--peer-id', peer, '--stake', stake, '--service-pubkey', r.id.service_pubkey, '--network', 'devnet', '--out', out, '--faucet-ckb', '200']);
  assert.notEqual(bad.code, 0, '--faucet-ckb must be gone');
});

test('the docs ask for about 700 CKB', () => {
  for (const f of ['README.md', 'docs/testing.md', 'docs/milestones/A8.md']) {
    const t = read(f);
    assert.doesNotMatch(t, /about 500 CKB/i, `${f} still says about 500 CKB`);
    assert.doesNotMatch(t, /161 CKB/, `${f} still mentions 161 CKB`);
  }
  assert.match(read('README.md'), /700 CKB/);
  assert.match(read('docs/testing.md'), /700 CKB/);
});
