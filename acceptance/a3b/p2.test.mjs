import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { setup, runCsr, cleanup, startMockNode, tmpDir, baseConfig, writeConfig, freePort } from '../a1/lib.mjs';
import { csrDev, liveCell, rpc, DEPLOYMENTS, RPC } from './devnet.mjs';

after(cleanup);

test('csr-dev refuses a mainnet node', async () => {
  const m = await startMockNode({ chain: 'ckb' });
  const r = await csrDev(['--rpc', m.url, 'owner-lock-args']);
  assert.notEqual(r.code, 0);
  assert.match(r.stderr + r.stdout, /mainnet|ckb_dev|ckb_testnet|refus/i);
});

test('deploy-contracts deploys five scripts and the config cell', async () => {
  const r = await csrDev(['deploy-contracts', '--out', DEPLOYMENTS]);
  assert.equal(r.code, 0, r.stderr);
  const d = JSON.parse(fs.readFileSync(DEPLOYMENTS, 'utf8'));
  for (const name of ['float_lock', 'dual_lock', 'registry', 'job_type', 'job_lock']) {
    const s = d.scripts[name];
    assert.ok(s, `scripts.${name} missing`);
    assert.equal(s.hash_type, 'data2');
    const cell = await liveCell(s.tx_hash, s.index);
    assert.equal(cell.status, 'live', `${name} code cell not live`);
    assert.equal(cell.cell.data.hash, s.code_hash, `${name}: code_hash must be the data hash of the deployed binary`);
  }
  const cfg = await liveCell(d.config_cell.tx_hash, d.config_cell.index);
  assert.equal(cfg.status, 'live');
  assert.ok(cfg.cell.output.type, 'config cell needs a Type ID type script');
  assert.match(d.registry_type_hash, /^0x[0-9a-f]{64}$/);
  assert.match(d.monitor_pubkey, /^0x0[23][0-9a-f]{64}$/);
});

test('clock-update moves the clock forward', async () => {
  const a = await csrDev(['clock-update']);
  assert.equal(a.code, 0, a.stderr);
  const tip = await rpc('get_tip_header');
  assert.ok(Math.abs(a.json.clock_ms - Number(BigInt(tip.timestamp))) < 120000);
});

test('csr accepts devnet in debug builds and reports its identity', async () => {
  const dir = tmpDir('a3b-ident');
  const cfg = baseConfig({ rpcPort: 18114, dashPort: await freePort(), pingPort: await freePort() });
  cfg.network = 'devnet';
  cfg.devnet = { deployments: DEPLOYMENTS };
  const f = writeConfig(dir, cfg);
  const v = await runCsr(['--data-root', dir, '--config', f, 'config', 'validate']);
  assert.equal(v.code, 0, v.stderr);
  const c = await runCsr(['--data-root', dir, '--config', f, 'check', '--json']);
  assert.equal(JSON.parse(c.stdout).r2.state, 'Synced', c.stdout);
  const id = await runCsr(['--data-root', dir, '--config', f, 'identity', '--json']);
  assert.equal(id.code, 0, id.stderr);
  const j = JSON.parse(id.stdout);
  assert.equal(j.network, 'devnet');
  assert.match(j.service_pubkey, /^0x0[23][0-9a-f]{64}$/);
  assert.match(j.runner_lock_args, /^0x[0-9a-f]{40}$/);
  assert.match(j.runner_lock_hash, /^0x[0-9a-f]{64}$/);
  assert.equal(RPC, 'http://127.0.0.1:18114');
});

test('devnet section is rejected for testnet', async () => {
  const env = await setup('a3b-devsec', { cfg: (c) => { c.devnet = { deployments: DEPLOYMENTS }; return c; } });
  const v = await runCsr([...env.args, 'config', 'validate']);
  assert.equal(v.code, 1);
});
