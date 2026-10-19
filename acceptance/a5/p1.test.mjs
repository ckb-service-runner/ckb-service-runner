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

test('deployments/testnet.json is unchanged and consistent', () => {
  assert.equal(DEP.network, 'testnet');
  assert.equal(DEP.monitor_pubkey, TESTNET_PK);
  assert.equal(DEP.scripts.registry.code_hash, '0x47fee83ea91eabdfc8827045f63fe051b8fee5c05b9280a35f42f2e3597d3f36');
  assert.equal(DEP.config_cell.type_hash, '0x581bc7bfe6c621361d665f79452e4ce3c8644d134980c0d29ca5302cd6b37439');
  for (const s of ['float_lock', 'dual_lock', 'registry', 'job_type', 'job_lock']) {
    assert.equal(DEP.scripts[s].hash_type, 'data2', s);
  }
});

test('network defaults name the testnet monitor', () => {
  const t = fs.readFileSync(path.join(ROOT, 'config', 'networks', 'testnet.toml'), 'utf8');
  assert.match(t, new RegExp(`^monitor_url\\s*=\\s*"${TESTNET_URL.replace(/[.]/g, '\\.')}"\\s*$`, 'm'));
  assert.match(t, new RegExp(`^monitor_pubkey\\s*=\\s*"${TESTNET_PK}"\\s*$`, 'm'));
  assert.doesNotMatch(t, /monitor\.invalid/);
});

test('register status on testnet uses the shipped deployment', async () => {
  const chain = await startChainMock({ chain: 'ckb_testnet' });
  const r = await runCsr([...(await env('a5-reg', 'testnet', chain)), 'register', 'status', '--json']);
  assert.equal(r.code, 0, r.stderr);
  const j = JSON.parse(r.stdout.trim());
  assert.equal(j.registered, false);
  const reg = chain.searches.find((k) => k.script_type === 'type' && k.script.code_hash === DEP.scripts.registry.code_hash);
  assert.ok(reg, `no registry search in ${JSON.stringify(chain.searches)}`);
  assert.equal(reg.script.hash_type, 'data2');
  assert.equal(reg.script.args, DEP.config_cell.type_hash);
});

test('float status on testnet searches the shipped float lock', async () => {
  const chain = await startChainMock({ chain: 'ckb_testnet' });
  const r = await runCsr([...(await env('a5-float', 'testnet', chain)), 'float', 'status', '--json']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(chain.searches.some((k) => k.script.code_hash === DEP.scripts.float_lock.code_hash),
    `no float lock search in ${JSON.stringify(chain.searches)}`);
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
