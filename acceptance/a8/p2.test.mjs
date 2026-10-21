// A8 Phase 2 (no devnet): runner address, consent verification against the Neuron vector, sweep settings.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { tmpDir, freePort, baseConfig, writeConfig, runCsr, cleanup } from '../a1/lib.mjs';
import { secpAddress, NEURON } from './addr.mjs';

after(cleanup);

async function env(name, network = 'testnet', payout = undefined) {
  const dir = tmpDir(name);
  const cfg = baseConfig({ rpcPort: await freePort(), dashPort: await freePort(), pingPort: await freePort() });
  cfg.network = network;
  if (payout) cfg.payout = payout;
  return ['--data-root', dir, '--config', writeConfig(dir, cfg)];
}

test('identity shows the runner address', async () => {
  for (const network of ['testnet', 'mainnet']) {
    const r = await runCsr([...(await env(`a8-id-${network}`, network)), 'identity', '--json']);
    assert.equal(r.code, 0, r.stderr);
    const id = JSON.parse(r.stdout.trim());
    assert.equal(id.runner_address, secpAddress(id.runner_lock_args, network));
    assert.ok(id.runner_address.startsWith(network === 'mainnet' ? 'ckb1q' : 'ckt1q'));
  }
});

test('verify-consent recovers the Neuron signer', async () => {
  const args = await env('a8-verify');
  const r = await runCsr([...args, 'register', 'verify-consent', '--message', NEURON.message, '--signature', NEURON.signature, '--json']);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(JSON.parse(r.stdout.trim()), { lock_args: NEURON.lock_args, address: NEURON.address });
  const other = await runCsr([...args, 'register', 'verify-consent', '--message', 'Message from CSR.', '--signature', NEURON.signature, '--json']);
  assert.equal(other.code, 0, other.stderr);
  assert.notEqual(JSON.parse(other.stdout.trim()).lock_args, NEURON.lock_args);
  for (const bad of ['0x1234', NEURON.signature.slice(0, -2), 'zz' + NEURON.signature.slice(4)]) {
    const b = await runCsr([...args, 'register', 'verify-consent', '--message', NEURON.message, '--signature', bad, '--json']);
    assert.notEqual(b.code, 0, bad);
    assert.match(b.stderr, /signature/i);
  }
});

test('sweep settings are validated', async () => {
  const ok = await runCsr([...(await env('a8-sweep-ok', 'testnet',
    { sweep_address: NEURON.address, sweep_threshold_ckb: 1500, sweep_amount_ckb: 1000 })), 'config', 'validate']);
  assert.equal(ok.code, 0, ok.stderr);
  const def = await runCsr([...(await env('a8-sweep-def')), 'config', 'validate']);
  assert.equal(def.code, 0, def.stderr);
  // the A7 key payout.lock_args is still accepted (and ignored)
  const old = await runCsr([...(await env('a8-sweep-old', 'testnet', { lock_args: NEURON.lock_args })), 'config', 'validate']);
  assert.equal(old.code, 0, old.stderr);

  const bad = [
    [{ sweep_address: NEURON.address }, 'mainnet', /payout\.sweep_address/],                       // testnet address on mainnet
    [{ sweep_address: NEURON.address.slice(0, -1) + (NEURON.address.endsWith('q') ? 'p' : 'q') }, 'testnet', /payout\.sweep_address/],
    [{ sweep_address: '0x89c04afd2ded4725e915020cb757a20d353f1094' }, 'testnet', /payout\.sweep_address/],
    [{ sweep_amount_ckb: 60 }, 'testnet', /payout\.sweep_amount_ckb/],
    [{ sweep_threshold_ckb: 1050, sweep_amount_ckb: 1000 }, 'testnet', /payout\.sweep_threshold_ckb/],
  ];
  let n = 0;
  for (const [payout, network, re] of bad) {
    const r = await runCsr([...(await env(`a8-sweep-bad-${n++}`, network, payout)), 'config', 'validate']);
    assert.notEqual(r.code, 0, JSON.stringify(payout));
    assert.match(r.stderr + r.stdout, re, JSON.stringify(payout));
  }
});

test('the A7 payout-cell and wallet-signed registration commands are gone', async () => {
  const args = await env('a8-gone');
  for (const cmd of [['payout', 'status'], ['payout', 'prepare-create', '--ckb', '100'], ['register', 'prepare']]) {
    const r = await runCsr([...args, ...cmd]);
    assert.notEqual(r.code, 0, cmd.join(' '));
  }
});
