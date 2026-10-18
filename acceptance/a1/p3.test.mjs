import { describe, test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { setup, startCsr, runCsr, getJson, waitFor, sleep, cleanup, blockHash, writeConfig, startMockNode } from './lib.mjs';

after(cleanup);

const SEMVER = /^\d+\.\d+\.\d+/;
const pubFromPriv = (h) => {
  const e = crypto.createECDH('secp256k1');
  e.setPrivateKey(Buffer.from(h.trim(), 'hex'));
  return '0x' + e.getPublicKey('hex', 'compressed');
};
const keyPath = (dir, net, name) => path.join(dir, net, 'keys', name);

describe('running csr on testnet', () => {
  let env;
  let csr;
  const seen = [];
  const api = async (p) => {
    const r = await getJson(env.base + p);
    seen.push(r.text);
    return r;
  };

  before(async () => {
    env = await setup('p3-run', { cfg: (c) => { c.logging.level = 'debug'; return c; } });
    csr = await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  });

  test('starts quickly', () => {
    assert.ok(csr.startupMs < 5000, `startup took ${csr.startupMs} ms`);
  });

  test('status', async () => {
    const r = await waitFor(async () => {
      const x = await api('/api/status');
      return x.json && x.json.r2 && x.json.r2.state === 'Synced' && x;
    });
    const s = r.json;
    assert.equal(s.network, 'testnet');
    assert.match(s.version, SEMVER);
    assert.equal(typeof s.uptime_s, 'number');
    assert.equal(s.peer_id, env.mock.state.nodeId);
    assert.match(s.service_pubkey, /^0x0[23][0-9a-f]{64}$/);
    assert.deepEqual(s.r2.reasons, []);
    assert.equal(s.runtime.state, 'off');
    assert.equal(s.mainnet_runtime_unlocked, false);
    assert.equal(s.ranking.join, false);
  });

  test('keys are created with mode 0600 and match the public key', async () => {
    for (const name of ['service.key', 'storage.key']) {
      const f = keyPath(env.dir, 'testnet', name);
      assert.ok(fs.existsSync(f), `${name} missing`);
      assert.equal(fs.statSync(f).mode & 0o777, 0o600, `${name} must be mode 0600`);
      assert.match(fs.readFileSync(f, 'utf8'), /^[0-9a-f]{64}\n?$/);
    }
    assert.equal(fs.statSync(path.join(env.dir, 'testnet', 'keys')).mode & 0o777, 0o700, 'keys/ must be mode 0700');
    const s = (await api('/api/status')).json;
    assert.equal(s.service_pubkey, pubFromPriv(fs.readFileSync(keyPath(env.dir, 'testnet', 'service.key'), 'utf8')));
  });

  test('node details without peer addresses', async () => {
    const r = await api('/api/node');
    assert.equal(r.status, 200);
    const n = r.json;
    assert.equal(n.chain, 'ckb_testnet');
    assert.equal(n.tip_number, 1000);
    assert.equal(n.tip_hash, blockHash(1000));
    assert.equal(n.best_known, 1000);
    assert.equal(n.tip_lag, 0);
    assert.equal(n.indexer_tip, 1000);
    assert.equal(n.indexer_lag, 0);
    assert.equal(n.peers_out, 1);
    assert.equal(n.peers_in, 1);
    assert.equal(n.tx_pool.pending, 2);
    assert.equal(n.tx_pool.proposed, 1);
    assert.ok(!r.text.includes('10.0.0.'), 'peer addresses must not be exposed');
  });

  test('resources and history', async () => {
    const r = await api('/api/resources');
    assert.equal(r.status, 200);
    assert.equal(typeof r.json.cpu.total_percent, 'number');
    assert.equal(typeof r.json.cpu.csr_percent, 'number');
    assert.ok(r.json.ram.total_mb > 0);
    assert.equal(typeof r.json.ram.free_mb, 'number');
    assert.equal(typeof r.json.disk.free_gb, 'number');
    const h = await waitFor(async () => {
      const x = await api('/api/resources/history');
      return x.json && Array.isArray(x.json.samples) && x.json.samples.length >= 2 && x;
    });
    for (const s of h.json.samples) assert.equal(typeof s.t, 'number');
  });

  test('R2 follows the node', async () => {
    env.mock.set({ ibd: true });
    await waitFor(async () => (await api('/api/status')).json.r2.state === 'Syncing', 6000);
    env.mock.set({ ibd: false, indexerTip: 900 });
    await waitFor(async () => (await api('/api/status')).json.r2.state === 'Degraded', 6000);
    env.mock.set({ indexerTip: 1000 });
    await waitFor(async () => (await api('/api/status')).json.r2.state === 'Synced', 6000);
  });

  test('state DB and logs exist; secrets never appear', async () => {
    assert.ok(fs.existsSync(path.join(env.dir, 'testnet', 'csr.db')), 'csr.db missing');
    const logFile = path.join(env.dir, 'testnet', 'logs', 'csr.log');
    assert.ok(fs.existsSync(logFile) && fs.statSync(logFile).size > 0, 'log file missing or empty');
    const logs = await api('/api/logs?level=debug');
    assert.equal(logs.status, 200);
    assert.ok(Array.isArray(logs.json.lines) && logs.json.lines.length <= 1000);
    const secrets = ['service.key', 'storage.key'].map((k) => fs.readFileSync(keyPath(env.dir, 'testnet', k), 'utf8').trim());
    const haystack = fs.readFileSync(logFile, 'utf8') + seen.join('\n') + csr.out.stdout + csr.out.stderr;
    for (const s of secrets) assert.ok(!haystack.includes(s), 'a private key leaked into logs or API output');
  });

  test('unreachable node keeps csr running', async () => {
    await env.mock.close();
    await waitFor(async () => (await api('/api/status')).json.r2.state === 'Unreachable', 6000);
    assert.equal((await fetch(`${env.base}/healthz`)).status, 200);
  });

  test('graceful stop; keys persist across restarts', async () => {
    const before = (await api('/api/status')).json.service_pubkey;
    const e = await csr.stop();
    assert.equal(e.code, 0, `SIGTERM must exit 0, got ${JSON.stringify(e)}`);
    const mock2 = await startMockNode();
    writeConfig(env.dir, { ...env.config, node: { ...env.config.node, rpc_url: mock2.url } });
    const csr2 = await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
    const s = (await getJson(`${env.base}/api/status`)).json;
    assert.equal(s.service_pubkey, before);
    await csr2.stop();
  });
});

test('refuses to start on a chain mismatch and prints both chains', async () => {
  const env = await setup('p3-chain', { node: { chain: 'ckb' } });
  const r = await runCsr([...env.args, 'run'], { timeoutMs: 20000 });
  assert.ok(r.code !== null && r.code !== 0, `expected a refusal, got ${r.code}`);
  assert.match(r.stderr, /ckb_testnet/);
  assert.match(r.stderr, /\bckb\b/);
});

test('refuses a light client', async () => {
  const env = await setup('p3-light', { node: { light: true } });
  const r = await runCsr([...env.args, 'run'], { timeoutMs: 20000 });
  assert.ok(r.code !== null && r.code !== 0, `expected a refusal, got ${r.code}`);
  assert.match(r.stderr, /unsupported|light/i);
});

test('mainnet needs --confirm-mainnet once; runtime stays inactive; data separated', async () => {
  const env = await setup('p3-main', {
    node: { chain: 'ckb' },
    cfg: (c) => { c.network = 'mainnet'; c.runtime.enabled = true; return c; },
  });
  const r = await runCsr([...env.args, 'run'], { timeoutMs: 20000 });
  assert.ok(r.code !== null && r.code !== 0, 'mainnet must not start without confirmation');
  assert.match(r.stderr, /--confirm-mainnet/);
  const a = await startCsr([...env.args, 'run', '--confirm-mainnet'], { healthUrl: `${env.base}/healthz` });
  const s = (await getJson(`${env.base}/api/status`)).json;
  assert.equal(s.network, 'mainnet');
  assert.equal(s.mainnet_runtime_unlocked, false);
  assert.equal(s.runtime.state, 'inactive');
  assert.ok(fs.existsSync(keyPath(env.dir, 'mainnet', 'service.key')));
  assert.ok(!fs.existsSync(path.join(env.dir, 'testnet')), 'a mainnet run must not create testnet data');
  await a.stop();
  const b = await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  await b.stop();
});

test('RPC load stays at or below 1 call per second', async () => {
  const env = await setup('p3-load');
  const csr = await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz`, env: { CSR_TEST_FAST_TICK: '0' } });
  await sleep(3000);
  const c0 = env.mock.calls;
  await sleep(30000);
  const used = env.mock.calls - c0;
  assert.ok(used <= 30, `${used} JSON-RPC calls in 30 s`);
  await csr.stop();
});
