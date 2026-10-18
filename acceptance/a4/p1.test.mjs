import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { ROOT, setup, getJson, sleep, cleanup, tmpDir } from '../a1/lib.mjs';

after(cleanup);

const REL = path.join(ROOT, 'target', 'release', 'csr');
const live = new Set();
after(() => { for (const p of live) { try { p.kill('SIGKILL'); } catch { /* ignore */ } } });

async function startRelease(args, healthUrl) {
  const started = Date.now();
  const p = spawn(REL, args, { stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, CSR_TEST_FAST_TICK: '1' } });
  live.add(p);
  let err = '';
  p.stderr.on('data', (d) => { err += d; });
  let exited = false;
  p.on('exit', () => { exited = true; live.delete(p); });
  for (;;) {
    if (exited) throw new Error(`release csr exited early:\n${err}`);
    if (Date.now() - started > 20000) throw new Error(`release csr not healthy:\n${err}`);
    try { if ((await fetch(healthUrl)).ok) break; } catch { /* not up yet */ }
    await sleep(50);
  }
  return { p, startupMs: Date.now() - started };
}
const rssMb = (pid) => {
  const m = fs.readFileSync(`/proc/${pid}/status`, 'utf8').match(/^VmRSS:\s+(\d+)\s+kB/m);
  return Number(m[1]) / 1024;
};

test('release binary size', () => {
  assert.ok(fs.existsSync(REL), 'target/release/csr missing');
  const mb = fs.statSync(REL).size / 1024 / 1024;
  assert.ok(mb < 30, `release binary is ${mb.toFixed(1)} MB`);
});

test('release build rejects devnet', () => {
  const dir = tmpDir('a4-devnet');
  const f = path.join(dir, 'csr.toml');
  fs.writeFileSync(f, 'network = "devnet"\n');
  const r = spawnSync(REL, ['--data-root', dir, '--config', f, 'config', 'validate'], { encoding: 'utf8' });
  assert.equal(r.status, 1, 'devnet must not exist in release builds');
});

test('startup under 3 s and idle memory under 50 MB', async () => {
  const env = await setup('a4-res');
  const { p, startupMs } = await startRelease([...env.args, 'run'], `${env.base}/healthz`);
  assert.ok(startupMs < 3000, `startup took ${startupMs} ms`);
  await sleep(15000);
  const mb = rssMb(p.pid);
  assert.ok(mb < 50, `idle RSS is ${mb.toFixed(1)} MB`);
  const s = (await getJson(`${env.base}/api/status`)).json;
  assert.equal(s.r2.state, 'Synced');
  assert.ok(s.uptime_s >= 10);
  p.kill('SIGTERM');
});

test('mainnet release: dashboard works, runtime inactive', async () => {
  const env = await setup('a4-main', { node: { chain: 'ckb' }, cfg: (c) => { c.network = 'mainnet'; c.runtime.enabled = true; return c; } });
  const { p } = await startRelease([...env.args, 'run', '--confirm-mainnet'], `${env.base}/healthz`);
  const s = (await getJson(`${env.base}/api/status`)).json;
  assert.equal(s.network, 'mainnet');
  assert.equal(s.runtime.state, 'inactive');
  assert.equal(s.mainnet_runtime_unlocked, false);
  assert.equal((await fetch(`${env.base}/`)).status, 200);
  p.kill('SIGTERM');
});

test('package, unit file and install docs', () => {
  const dist = path.join(ROOT, 'dist');
  const tarball = fs.readdirSync(dist).find((f) => /^csr-\d+\.\d+\.\d+-linux-x86_64\.tar\.gz$/.test(f));
  assert.ok(tarball, 'tarball missing in dist/');
  const list = spawnSync('tar', ['-tzf', path.join(dist, tarball)], { encoding: 'utf8' }).stdout;
  for (const f of ['csr', 'csr.example.toml', 'csr.service', 'README.md', 'INSTALL.md']) {
    assert.ok(list.split('\n').some((l) => l === f || l.endsWith(`/${f}`)), `${f} missing in the tarball`);
  }
  assert.ok(!list.includes('csr-dev'), 'csr-dev must not be packaged');
  assert.match(fs.readFileSync(path.join(dist, 'SHA256SUMS'), 'utf8'), new RegExp(tarball.replace(/\./g, '\\.')));
  assert.match(fs.readFileSync(path.join(ROOT, '.gitignore'), 'utf8'), /dist/);

  const unit = fs.readFileSync(path.join(ROOT, 'config', 'csr.service'), 'utf8');
  for (const d of ['User=csr', 'ExecStart=/usr/local/bin/csr', 'Restart=on-failure', 'Nice=19', 'IOSchedulingClass=idle',
    'NoNewPrivileges=yes', 'ProtectSystem=strict', 'ProtectHome=yes', 'PrivateTmp=yes', 'ReadWritePaths=/var/lib/csr', 'MemoryMax=']) {
    assert.ok(unit.includes(d), `csr.service must contain ${d}`);
  }
  const install = fs.readFileSync(path.join(ROOT, 'docs', 'install.md'), 'utf8');
  for (const s of ['Indexer', 'csr detect', 'csr check', 'systemctl', 'ssh -L', 'set-password', '8131', 'outbound', '--confirm-mainnet']) {
    assert.ok(install.includes(s), `docs/install.md must mention ${s}`);
  }
  assert.ok(fs.existsSync(path.join(ROOT, 'docs', 'milestones', 'A4.md')));
});
