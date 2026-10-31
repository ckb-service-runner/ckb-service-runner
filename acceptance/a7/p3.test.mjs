// A7 Phase 3: registration with a DAO stake and a payout cell; `register attest --stake`.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { tmpDir, freePort, baseConfig, writeConfig, runCsr, startMockNode, cleanup } from '../a1/lib.mjs';
import { verify, bindingMsg } from '../a1/crypto.mjs';
import { startAttestMonitor } from '../a5/attestmon.mjs';
import { V } from '../a5/vectors.mjs';
import { rpc, csrJson } from '../a3b/devnet.mjs';
import { fundRunnerCell, signAndSubmit } from '../a3b/runner.mjs';
import { CKB, deploy, runner, deposit, createPayout, prepareRegister, payoutArgs } from './flow.mjs';

after(cleanup);

// ---------- register attest (no devnet) ----------

async function attestEnv(name, monUrl) {
  const dir = tmpDir(name);
  const keys = path.join(dir, 'testnet', 'keys');
  fs.mkdirSync(keys, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(keys, 'service.key'), V.service_key + '\n', { mode: 0o600 });
  const node = await startMockNode({ chain: 'ckb_testnet' });
  const cfg = baseConfig({ rpcPort: node.port, dashPort: await freePort(), pingPort: await freePort() });
  cfg.monitor = { url: monUrl, pubkey: V.monitor_pubkey };
  return { args: ['--data-root', dir, '--config', writeConfig(dir, cfg)], out: path.join(dir, 'att', 'attestation.json') };
}

const STAKE = '0x' + 'ab'.repeat(32) + ':2';

test('register attest: the stake is required and checked before asking the monitor', async () => {
  const mon = await startAttestMonitor(() => [200, V.ok]);
  const e = await attestEnv('a7-att-bad', mon.url);
  const none = await runCsr([...e.args, 'register', 'attest', '--out', e.out, '--json']);
  assert.notEqual(none.code, 0, '--stake is required');
  for (const bad of ['0x12:0', '0x' + 'ab'.repeat(32), '0x' + 'ab'.repeat(32) + ':x', 'ab'.repeat(32) + ':0']) {
    const r = await runCsr([...e.args, 'register', 'attest', '--stake', bad, '--out', e.out, '--json']);
    assert.notEqual(r.code, 0, bad);
    assert.match(r.stderr, /stake/i);
  }
  assert.equal(mon.requests.length, 0, 'the monitor must not be asked with a malformed stake');
});

test('register attest reports stake errors of the monitor', async () => {
  const mon = await startAttestMonitor(() => [403, { error: 'stake_invalid', reason: 'too_small' }]);
  const e = await attestEnv('a7-att-403', mon.url);
  const r = await runCsr([...e.args, 'register', 'attest', '--stake', STAKE, '--out', e.out, '--json']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /stake_invalid/);
  assert.match(r.stderr, /too_small/);
  assert.equal(fs.existsSync(e.out), false);
});

// ---------- devnet ----------
