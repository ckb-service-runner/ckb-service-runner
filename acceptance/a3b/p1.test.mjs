import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { spawnSync } from 'node:child_process';
import { rpc, mine, tipNumber } from './devnet.mjs';

test('devnet answers, is a dev chain and mines on demand', async () => {
  const info = await rpc('get_blockchain_info');
  assert.equal(info.chain, 'ckb_dev');
  assert.equal(info.is_initial_block_download, false);
  const t0 = await tipNumber();
  mine(3);
  assert.equal(await tipNumber(), t0 + 3);
  const idx = await rpc('get_indexer_tip');
  assert.ok(idx && idx.block_number, 'Indexer module must be enabled');
});

test('start is idempotent and status works', () => {
  assert.equal(spawnSync('bash', ['scripts/devnet.sh', 'start']).status, 0);
  assert.equal(spawnSync('bash', ['scripts/devnet.sh', 'status']).status, 0);
});

test('devnet files are not committed', () => {
  const gi = fs.readFileSync('.gitignore', 'utf8');
  for (const p of ['.tools', '.devnet', 'deployments/devnet.json']) assert.ok(gi.includes(p), `.gitignore must contain ${p}`);
  assert.ok(fs.existsSync('devtools/devnet-keys.json'));
  assert.ok(fs.existsSync('docs/testing.md'));
});
