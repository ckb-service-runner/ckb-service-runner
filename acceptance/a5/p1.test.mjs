// A5 Phase 1: testnet deployments shipped in the binary, real testnet monitor defaults.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, tmpDir, freePort, baseConfig, writeConfig, runCsr, cleanup } from '../a1/lib.mjs';
import { startChainMock } from './chainmock.mjs';

after(cleanup);

const DEP = JSON.parse(fs.readFileSync(path.join(ROOT, 'deployments', 'testnet.json'), 'utf8'));
const TESTNET_URL = 'https://monitor-testnet.vericell.net';
const TESTNET_PK = '0x03db2a5f9af76cf37294137ec5d5533abf014b6c399cd4ca0722778ffa07980c7c';

async function env(name, network, chain) {
  const dir = tmpDir(name);
  const cfg = baseConfig({ rpcPort: chain ? chain.port : await freePort(), dashPort: await freePort(), pingPort: await freePort() });
  cfg.network = network;
  const f = writeConfig(dir, cfg);
  return ['--data-root', dir, '--config', f];
}

test('network defaults name the testnet monitor', () => {
  const t = fs.readFileSync(path.join(ROOT, 'config', 'networks', 'testnet.toml'), 'utf8');
  assert.match(t, new RegExp(`^monitor_url\\s*=\\s*"${TESTNET_URL.replace(/[.]/g, '\\.')}"\\s*$`, 'm'));
  assert.match(t, new RegExp(`^monitor_pubkey\\s*=\\s*"${TESTNET_PK}"\\s*$`, 'm'));
  assert.doesNotMatch(t, /monitor\.invalid/);
});

test('testnet chain commands refuse a node on another chain', async () => {
  const chain = await startChainMock({ chain: 'ckb' });
  const r = await runCsr([...(await env('a5-wrong', 'testnet', chain)), 'register', 'status', '--json']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /ckb_testnet/);
});

test('mainnet is still not deployed', async () => {
  const chain = await startChainMock({ chain: 'ckb' });
  const r = await runCsr([...(await env('a5-main', 'mainnet', chain)), 'register', 'status', '--json']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /not deployed on mainnet/);
});
