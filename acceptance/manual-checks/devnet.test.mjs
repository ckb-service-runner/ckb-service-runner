// Devnet versions of the manual checks of the Oct 2026 audit (specs 03, 04, 09, 17).
// Run with scripts/manual-checks.sh (builds, starts the devnet, runs this file, stops the devnet).
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { cleanup, startCsr, getJson, waitFor, freePort, onCleanup, runCsr, ROOT } from '../a1/lib.mjs';
import { csrDev, csrJson, liveCell, waitCommitted, mine, rpc } from '../a3b/devnet.mjs';
import { deployService, allow, startMiner } from '../a3c/jobs.mjs';
import { deploy, deposit } from '../a7/flow.mjs';
import { runner, fundRunnerCell, registerWithConsent, signMessage } from '../a8/flow.mjs';
import { runService, asciiHex } from '../a2/svc.mjs';
import { skeletonSpending, watchPayload } from './molecule.mjs';

after(cleanup);

const SKELETON_ELF = path.join(ROOT, 'services', 'c', 'skeletontest', 'build', 'skeletontest');

async function job(codeHash, { payload = '0x', tipCkb = 5 } = {}) {
  const r = await csrDev(['job', '--code-hash', codeHash, '--tip-ckb', String(tipCkb), '--max-fee-shannons', '100000',
    '--max-cycles', '10000000', '--trigger', 'ondemand', '--runs', '1', '--payload', payload]);
  assert.equal(r.code, 0, r.stderr);
  await waitCommitted(r.json.tx_hash);
  return `${r.json.tx_hash}:${r.json.index}`;
}

async function fundCell(lockArgs, ckb) {
  const f = await csrDev(['fund', '--lock-args', lockArgs, '--ckb', String(ckb)]);
  assert.equal(f.code, 0, f.stderr);
  await waitCommitted(f.json.tx_hash);
  return `${f.json.tx_hash}:0`;
}

// Watched cells belong to the devnet monitor test key, not to the funding key: csr-dev spends the
// funding key's cells when it builds jobs and deposits, which would consume them by accident.
const SPARE = { privkey: '0x5d1b2c3e4f60718293a4b5c6d7e8f9012a3b4c5d6e7f8091a2b3c4d5e6f70812', lock_args: '0x9a8d9aa937744fbfaa0a21d0eb83e3600c6a9134' };
async function spendSpare(op) {
  const x = await csrDev(['reclaim', '--outpoint', op, '--key', SPARE.privkey]);
  assert.equal(x.code, 0, x.stderr);
  await waitCommitted(x.json.tx_hash);
}

const isLive = async (op) => {
  const [h, i] = op.split(':');
  return (await liveCell(h, Number(i))).status === 'live';
};

async function jobs(r) {
  return (await getJson(`${r.base}/api/jobs`)).json.jobs;
}

async function jobDetail(r, id) {
  return (await getJson(`${r.base}/api/jobs/${id}`)).json;
}

async function registeredRunner(name, mutate = (c) => c) {
  const r = await runner(name, undefined, (c) => { c.runtime.enabled = true; return mutate(c); });
  await fundRunnerCell(r, 700);
  await registerWithConsent(r, await deposit(10000));
  return r;
}

// ---------------------------------------------------------------- spec 03: skeleton guard
test('spec 03: a skeleton spending a runner-owned cell fails before signing; a normal job settles', async () => {
  await deploy();
  const sk = await deployService('heartbeat');           // normal service, empty skeleton
  const bad = await csrDev(['deploy-service', '--elf', SKELETON_ELF]);
  assert.equal(bad.code, 0, bad.stderr);
  await waitCommitted(bad.json.tx_hash);
  const r = await registeredRunner('mc-skeleton');
  await allow(r, sk);
  await allow(r, bad.json);
  const reg = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.equal(reg.registered, true);
  const walletCell = await fundCell(r.id.runner_lock_args, 100);

  const jobReg = await job(bad.json.code_hash, { payload: skeletonSpending([reg.outpoint]) });
  const jobWallet = await job(bad.json.code_hash, { payload: skeletonSpending([walletCell]) });
  const jobOk = await job(sk.code_hash);

  await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const stop = startMiner(500);
  try {
    await waitFor(async () => {
      const js = await jobs(r);
      const by = (op) => js.find((j) => j.outpoint === op);
      return by(jobReg)?.state === 'Failed' && by(jobWallet)?.state === 'Failed' && by(jobOk)?.state === 'Settled';
    }, 240000, 1000);
  } finally {
    await stop();
  }
  const js = await jobs(r);
  for (const [op, why] of [[jobReg, /dual lock naming the runner|registry cell/], [jobWallet, /runner wallet/]]) {
    const j = js.find((x) => x.outpoint === op);
    assert.equal(j.result_tx, null, 'no transaction was sent');
    const d = await jobDetail(r, j.id);
    const note = d.timeline.map((t) => t.note || '').join('\n');
    assert.match(note, /skeleton input 0/);
    assert.match(note, why);
  }
  assert.ok(await isLive(reg.outpoint), 'the registry cell is still live');
  assert.ok(await isLive(walletCell), 'the runner wallet cell is still live');
  assert.equal((await csrJson([...r.args, 'register', 'status'])).json.outpoint, reg.outpoint);
  assert.match(js.find((x) => x.outpoint === jobOk).result_tx, /^0x[0-9a-f]{64}$/);
});

// ---------------------------------------------------------------- spec 04: delete-on-consume
test('spec 04: spending a real cell deletes its entry, also with the service disabled', async () => {
  await deploy();
  const r = await runner('mc-doc', undefined, (c) => { c.runtime.enabled = true; return c; });
  const add = await runCsr([...r.args, 'service', 'add-local', '--manifest', path.join(ROOT, 'services', 'manifests', 'dottest.toml')]);
  assert.equal(add.code, 0, add.stderr);
  const dot = JSON.parse(add.stdout).code_hash;
  assert.equal((await runCsr([...r.args, 'service', 'approve', dot])).code, 0);

  const store = async (op) => {
    const [h, i] = op.split(':');
    const idx = Buffer.alloc(4);
    idx.writeUInt32LE(Number(i));
    const s = await runService(r, 'dottest', '0x01' + h.slice(2) + idx.toString('hex'));
    assert.equal(s.status, 'ok', JSON.stringify(s));
    assert.equal(s.result_hex, asciiHex('stored'));
    assert.equal((await runService(r, 'dottest', '0x02')).result_hex, asciiHex('present'));
  };
  const reclaim = spendSpare;
  let proc;
  const log = () => (proc ? proc.out.stdout + proc.out.stderr : '');

  // enabled service
  const c1 = await fundCell(SPARE.lock_args, 100);
  await store(c1);
  proc = await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  await reclaim(c1);
  mine(2);
  await waitFor(() => /delete_on_consume/.test(log()), 60000, 500).catch(() => {});
  const log1 = log();
  await proc.stop();
  assert.equal((await runService(r, 'dottest', '0x02')).result_hex, asciiHex('absent'), 'entry deleted by the watcher');
  assert.match(log1, /delete_on_consume/);

  // disabled service
  const c2 = await fundCell(SPARE.lock_args, 100);
  await store(c2);
  assert.equal((await runCsr([...r.args, 'service', 'disable', dot])).code, 0);
  proc = await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  await reclaim(c2);
  mine(2);
  await waitFor(() => /delete_on_consume/.test(log()), 60000, 500).catch(() => {});
  await proc.stop();
  assert.equal((await runService(r, 'dottest', '0x02')).result_hex, asciiHex('absent'), 'entry deleted with the service disabled');
});

// ---------------------------------------------------------------- spec 17: cell-consumed trigger
test('spec 17: a watched job waits for its cell, runs once it is spent; a normal job runs at once', async () => {
  await deploy();
  const hb = await deployService('heartbeat');
  const r = await registeredRunner('mc-watch');
  await allow(r, hb);
  const c = await fundCell(SPARE.lock_args, 100);
  const watched = await job(hb.code_hash, { payload: watchPayload(c) });
  const normal = await job(hb.code_hash);

  await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  let stop = startMiner(500);
  try {
    await waitFor(async () => (await jobs(r)).find((j) => j.outpoint === normal)?.state === 'Settled', 120000, 1000);
    await new Promise((res) => setTimeout(res, 10000));
  } finally {
    await stop();
  }
  let js = await jobs(r);
  const w = js.find((j) => j.outpoint === watched);
  assert.ok(!w || w.state !== 'Settled', `the watched job must wait, got ${JSON.stringify(w)}`);
  if (w) assert.equal(w.trigger, `on-cell-consumed:${c}`);

  assert.ok(await isLive(c), 'the watched cell is still live');
  await spendSpare(c);
  stop = startMiner(500);
  try {
    await waitFor(async () => (await jobs(r)).find((j) => j.outpoint === watched)?.state === 'Settled', 180000, 1000);
  } finally {
    await stop();
  }
  js = await jobs(r);
  assert.match(js.find((j) => j.outpoint === watched).result_tx, /^0x[0-9a-f]{64}$/);
});

// ---------------------------------------------------------------- spec 09: attestation renewal
// A devnet "monitor" answering POST /v1/attest with csr-dev attest (expiry chosen by the test).
async function startDevnetMonitor(port, expiryMs) {
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (d) => { body += d; });
    req.on('end', async () => {
      res.setHeader('content-type', 'application/json');
      if (req.method !== 'POST' || req.url !== '/v1/attest') { res.statusCode = 404; res.end('{"error":"not_found"}'); return; }
      const j = JSON.parse(body);
      const out = path.join(ROOT, 'target', `mc-att-${Date.now()}.json`);
      const a = await csrDev(['attest', '--peer-id', j.peer_id, '--service-pubkey', j.service_pubkey, '--network', 'devnet',
        '--expiry-ms', String(expiryMs()), '--out', out]);
      if (a.code !== 0) { res.statusCode = 500; res.end(JSON.stringify({ error: a.stderr })); return; }
      res.statusCode = 200;
      res.end(fs.readFileSync(out, 'utf8'));
    });
  });
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  const close = () => new Promise((r) => { server.closeAllConnections(); server.close(() => r()); });
  onCleanup(close);
  return { close };
}

const MONITOR_PUBKEY = '0x031ee69e71a83ca2ff16539d56188eacc7bac98f08448d03fce80f2f5a12b484a4'; // devnet test key

test('spec 09: automatic renewal near the expiry, error while the monitor is down, refresh by hand', async () => {
  await deploy();
  const clock = await csrDev(['clock-update']);
  assert.equal(clock.code, 0, clock.stderr);
  const port = await freePort();
  const DAY = 86400000;
  let lifetime = 1.5 * DAY;                  // inside the 2-day renewal window: due at once
  let mon = await startDevnetMonitor(port, () => Date.now() + lifetime);
  const r = await runner('mc-renew', undefined, (c) => {
    c.runtime.enabled = true;
    c.monitor = { url: `http://127.0.0.1:${port}`, pubkey: MONITOR_PUBKEY };
    return c;
  });
  await fundRunnerCell(r, 700);
  const stake = await deposit(10000);
  const m = await csrJson([...r.args, 'register', 'consent-message', '--stake', stake]);
  assert.equal(m.code, 0, m.stderr);
  const sig = await signMessage(m.json.message);
  const reg = await csrJson([...r.args, 'register', 'create', '--stake', stake, '--consent', sig.signature]);
  assert.equal(reg.code, 0, reg.stderr);                   // attestation from the (devnet) monitor
  await waitCommitted(reg.json.tx_hash);
  const exp = async () => (await csrJson([...r.args, 'register', 'status'])).json.attestation_expiry_ms;
  const st0 = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.ok(st0.attestation_expiry_ms < Date.now() + 2 * DAY);

  // monitor down: the automatic renewal fails, the dashboard shows the error
  await mon.close();
  const proc = await startCsr([...r.args, 'run'], { healthUrl: `${r.base}/healthz` });
  const stop = startMiner(500);
  const log = () => proc.out.stdout + proc.out.stderr;
  try {
    await waitFor(() => /attestation renewal failed/.test(log()), 60000, 500);
    await waitFor(async () => (await getJson(`${r.base}/api/registry`)).json?.renewal_error, 30000, 500);

    // monitor back: renewed by itself, later expiry, same stake, error cleared
    lifetime = 6 * DAY;
    mon = await startDevnetMonitor(port, () => Date.now() + lifetime);
    await waitFor(() => /attestation renewed in/.test(log()), 120000, 1000);
    await waitFor(async () => (await exp()) > Date.now() + 5 * DAY, 60000, 1000);
    await waitFor(async () => (await getJson(`${r.base}/api/registry`)).json?.renewal_error === null, 30000, 500);
  } finally {
    await stop();
    await proc.stop();
  }
  const st1 = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.deepEqual(st1.stake, st0.stake, 'same stake');
  assert.equal(st1.registered, true);

  // by hand: csr register refresh
  lifetime = 7 * DAY;
  const ref = await csrJson([...r.args, 'register', 'refresh']);
  assert.equal(ref.code, 0, ref.stderr);
  await waitCommitted(ref.json.tx_hash);
  const st2 = (await csrJson([...r.args, 'register', 'status'])).json;
  assert.ok(st2.attestation_expiry_ms > st1.attestation_expiry_ms, 'later expiry');
  assert.deepEqual(st2.stake, st0.stake, 'same stake');
});
