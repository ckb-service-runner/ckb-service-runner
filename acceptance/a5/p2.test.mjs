// A5 Phase 2: `csr register attest` requests and checks the monitor attestation.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { tmpDir, freePort, baseConfig, writeConfig, runCsr, startMockNode, cleanup } from '../a1/lib.mjs';
import { verify, bindingMsg } from '../a1/crypto.mjs';
import { startAttestMonitor } from './attestmon.mjs';
import { V } from './vectors.mjs';

after(cleanup);

// A data root whose testnet service key is V.service_key, a mock node and a config pointing at `mon`.
async function env(name, monUrl, { pubkey = V.monitor_pubkey, network = 'testnet' } = {}) {
  const dir = tmpDir(name);
  const keys = path.join(dir, network, 'keys');
  fs.mkdirSync(keys, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(keys, 'service.key'), V.service_key + '\n', { mode: 0o600 });
  const node = await startMockNode({ chain: network === 'mainnet' ? 'ckb' : 'ckb_testnet' });
  const cfg = baseConfig({ rpcPort: node.port, dashPort: await freePort(), pingPort: await freePort() });
  cfg.network = network;
  cfg.monitor = { url: monUrl, pubkey };
  const f = writeConfig(dir, cfg);
  return { dir, args: ['--data-root', dir, '--config', f], out: path.join(dir, 'att', 'attestation.json') };
}

test('fixtures', () => {
  assert.equal(V.peer_id, 'QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA');
});

test('a valid attestation is requested, checked and written', async () => {
  const mon = await startAttestMonitor(() => [200, V.ok]);
  const e = await env('a5-ok', mon.url);
  const r = await runCsr([...e.args, 'register', 'attest', '--out', e.out, '--json']);
  assert.equal(r.code, 0, r.stderr);
  // the request
  assert.equal(mon.requests.length, 1);
  const q = mon.requests[0];
  assert.deepEqual(Object.keys(q).sort(), ['binding_sig', 'network', 'peer_id', 'service_pubkey']);
  assert.equal(q.peer_id, V.peer_id);
  assert.equal(q.service_pubkey, V.service_pubkey);
  assert.equal(q.network, 'testnet');
  assert.ok(verify(V.service_pubkey, bindingMsg(V.peer_id, V.service_pubkey, 'testnet'), q.binding_sig), 'binding_sig');
  // the file is the monitor's JSON
  assert.deepEqual(JSON.parse(fs.readFileSync(e.out, 'utf8')), V.ok);
  const j = JSON.parse(r.stdout.trim());
  assert.equal(j.file, e.out);
  assert.equal(j.peer_id, V.peer_id);
  assert.equal(j.expiry_ms, V.ok.expiry_ms);
});

test('human output without --json', async () => {
  const mon = await startAttestMonitor(() => [200, V.ok]);
  const e = await env('a5-text', mon.url);
  const r = await runCsr([...e.args, 'register', 'attest', '--out', e.out]);
  assert.equal(r.code, 0, r.stderr);
  assert.match(r.stdout, /^expiry_ms: 4102444800000$/m);
  assert.match(r.stdout, new RegExp(`^peer_id: ${V.peer_id}$`, 'm'));
});

const bad = [
  ['not eligible', () => [403, { error: 'not_eligible' }], /not_eligible/],
  ['other monitor errors', () => [503, { error: 'chain_unavailable' }], /503/],
  ['signed by another key', () => [200, V.badsig], /monitor key/],
  ['expired', () => [200, V.expired], /expired/],
  ['for another peer', () => [200, V.otherpeer], /QmSomeOtherNode/],
  ['for another network', () => [200, V.mainnet], /network/],
  ['for another service key', () => [200, { ...V.ok, service_pubkey: V.other_pubkey }], /service key/],
  ['not JSON', () => [200, 'nope'], /./],
];
for (const [name, reply, err] of bad) {
  test(`rejected: ${name}`, async () => {
    const mon = await startAttestMonitor(reply);
    const e = await env(`a5-${name.replace(/\W+/g, '-')}`, mon.url);
    const r = await runCsr([...e.args, 'register', 'attest', '--out', e.out, '--json']);
    assert.equal(r.code, 1, r.stdout);
    assert.match(r.stderr, err);
    assert.equal(fs.existsSync(e.out), false, 'no file may be written');
  });
}

test('the configured monitor key decides', async () => {
  const mon = await startAttestMonitor(() => [200, V.ok]);
  const e = await env('a5-pk', mon.url, { pubkey: V.other_pubkey });
  const r = await runCsr([...e.args, 'register', 'attest', '--out', e.out, '--json']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /monitor key/);
});

test('monitor unreachable', async () => {
  const e = await env('a5-down', `http://127.0.0.1:${await freePort()}`);
  const r = await runCsr([...e.args, 'register', 'attest', '--out', e.out, '--json'], { timeoutMs: 30000 });
  assert.equal(r.code, 1);
  assert.ok(r.stderr.trim().length > 0);
  assert.equal(fs.existsSync(e.out), false);
});

test('node unreachable', async () => {
  const mon = await startAttestMonitor(() => [200, V.ok]);
  const e = await env('a5-nonode', mon.url);
  const cfgFile = e.args[3];
  const t = fs.readFileSync(cfgFile, 'utf8').replace(/rpc_url = "[^"]*"/, `rpc_url = "http://127.0.0.1:${await freePort()}"`);
  fs.writeFileSync(cfgFile, t);
  const r = await runCsr([...e.args, 'register', 'attest', '--out', e.out, '--json'], { timeoutMs: 30000 });
  assert.equal(r.code, 1);
  assert.equal(mon.requests.length, 0);
});

test('docs describe the testnet flow', () => {
  const read = (f) => fs.readFileSync(path.join(process.cwd(), f), 'utf8');
  const a5 = read('docs/milestones/A5.md');
  for (const s of ['csr register attest', 'deployments/testnet.json', 'monitor-testnet.vericell.net']) {
    assert.ok(a5.includes(s), `docs/milestones/A5.md lacks ${s}`);
  }
  const t = read('docs/testing.md');
  assert.ok(t.includes('csr register attest'), 'docs/testing.md lacks csr register attest');
  assert.ok(!t.includes('shipped values do not exist yet'), 'docs/testing.md still has the old TODO');
  assert.ok(read('CHANGELOG.md').includes('register attest'), 'CHANGELOG.md lacks the A5 entry');
});
