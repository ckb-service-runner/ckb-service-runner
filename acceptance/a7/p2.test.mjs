// A7 Phase 2: deployments without the float lock, the vendored anyone-can-pay lock, csr-dev dao-deposit,
// `csr payout …` (testnet search via a chain mock; full flow on the devnet).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { ROOT, tmpDir, freePort, baseConfig, writeConfig, runCsr, cleanup } from '../a1/lib.mjs';
import { startChainMock } from '../a5/chainmock.mjs';
import { csrJson, liveCell } from '../a3b/devnet.mjs';
import { signAndSubmit } from '../a3b/runner.mjs';
import { fileHash, scriptHash } from './ckb.mjs';
import { CKB, DAO_TYPE, ACP_DEVNET_CODE_HASH, ACP_TESTNET, deploy, owner, runner, deposit, createPayout } from './flow.mjs';

after(cleanup);

const ACP_FILE = path.join(ROOT, 'devtools', 'vendor', 'anyone_can_pay');
const ACP_SHA256 = '5eca558b873dcf54e589fa2135a7a0a13defcfd87469cac7dfc6a38cbf57720a';

test('the anyone-can-pay binary is vendored and pinned', () => {
  assert.ok(fs.existsSync(ACP_FILE), 'devtools/vendor/anyone_can_pay missing');
  const sha = crypto.createHash('sha256').update(fs.readFileSync(ACP_FILE)).digest('hex');
  assert.equal(sha, ACP_SHA256);
  assert.equal(fileHash(ACP_FILE), ACP_DEVNET_CODE_HASH);
  const sums = fs.readFileSync(path.join(ROOT, 'devtools', 'vendor', 'SHA256SUMS'), 'utf8');
  assert.match(sums, new RegExp(`^${ACP_SHA256}\\s+\\*?anyone_can_pay$`, 'm'));
});

test('dao-deposit creates a Nervos DAO deposit of the genesis key', async () => {
  await deploy();
  const o = await owner();
  const op = await deposit(10000);
  const [tx, idx] = op.split(':');
  const c = await liveCell(tx, Number(idx));
  assert.equal(c.status, 'live');
  assert.deepEqual(c.cell.output.type, DAO_TYPE);
  assert.equal(c.cell.data.content, '0x0000000000000000');
  assert.equal(Number(BigInt(c.cell.output.capacity)), 10000 * CKB);
  assert.equal(c.cell.output.lock.args, o.lock_args);
  assert.equal(scriptHash(c.cell.output.lock), o.lock_hash);
});
