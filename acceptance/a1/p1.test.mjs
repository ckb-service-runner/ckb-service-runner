import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, runCsr, tmpDir, baseConfig, toToml, cleanup } from './lib.mjs';

after(cleanup);

async function validate(text) {
  const dir = tmpDir('p1');
  const f = path.join(dir, 'csr.toml');
  fs.writeFileSync(f, text);
  const r = await runCsr(['--data-root', dir, '--config', f, 'config', 'validate']);
  return { ...r, dir };
}
const good = () => baseConfig({ rpcPort: 18114, dashPort: 18130, pingPort: 18131 });

test('csr version prints a semver', async () => {
  const r = await runCsr(['version']);
  assert.equal(r.code, 0, r.stderr);
  assert.match(r.stdout, /^csr \d+\.\d+\.\d+/m);
});

test('example config exists and is valid', async () => {
  const f = path.join(ROOT, 'config', 'csr.example.toml');
  assert.ok(fs.existsSync(f), 'config/csr.example.toml missing');
  const r = await validate(fs.readFileSync(f, 'utf8'));
  assert.equal(r.code, 0, r.stderr);
});

test('network files exist', () => {
  for (const n of ['testnet', 'mainnet']) {
    assert.ok(fs.existsSync(path.join(ROOT, 'config', 'networks', `${n}.toml`)), `config/networks/${n}.toml missing`);
  }
});

test('a complete valid config passes and validation creates no files', async () => {
  const r = await validate(toToml(good()));
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(fs.readdirSync(r.dir), ['csr.toml']);
});

test('localhost forms of rpc_url are accepted', async () => {
  for (const url of ['http://localhost:8114', 'http://[::1]:8114', 'http://127.0.0.1:9999']) {
    const c = good();
    c.node.rpc_url = url;
    const r = await validate(toToml(c));
    assert.equal(r.code, 0, `${url}: ${r.stderr}`);
  }
});

test('public dashboard with a password hash is accepted', async () => {
  const c = good();
  c.dashboard.bind = '0.0.0.0:18130';
  c.dashboard.password_hash = '$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$Zm9vYmFyZm9vYmFyZm9vYmFyZm9vYmFyZm9vYmFyZm8';
  const r = await validate(toToml(c));
  assert.equal(r.code, 0, r.stderr);
});

const invalid = [
  ['remote rpc_url', (c) => { c.node.rpc_url = 'http://192.168.1.10:8114'; }, /localhost/i],
  ['https remote rpc_url', (c) => { c.node.rpc_url = 'https://example.com:8114'; }, null],
  ['unknown network', (c) => { c.network = 'regtest'; }, null],
  ['negative min_ckb_per_mcycle', (c) => { c.runtime.min_ckb_per_mcycle = -1; }, null],
  ['zero max_concurrent_jobs', (c) => { c.limits.max_concurrent_jobs = 0; }, null],
  ['per-service quota above total', (c) => { c.storage.per_service_quota_mb = 512; }, null],
  ['public dashboard without password', (c) => { c.dashboard.bind = '0.0.0.0:18130'; }, /password/i],
  ['plain-http monitor on a remote host', (c) => { c.monitor.url = 'http://10.1.2.3:9000'; }, /https/i],
  ['malformed monitor pubkey', (c) => { c.monitor.pubkey = '0x1234'; }, null],
  ['bad ping_mode', (c) => { c.ranking.ping_mode = 'sideways'; }, null],
  ['bad log level', (c) => { c.logging.level = 'loud'; }, null],
  ['bad node_type', (c) => { c.node.node_type = 'light'; }, null],
];
for (const [name, mutate, re] of invalid) {
  test(`rejects ${name}`, async () => {
    const c = good();
    mutate(c);
    const r = await validate(toToml(c));
    assert.equal(r.code, 1, `expected exit 1, got ${r.code}; stderr: ${r.stderr}`);
    assert.ok(r.stderr.trim().length > 0, 'a reason must be printed on stderr');
    if (re) assert.match(r.stderr, re);
  });
}

test('unknown keys are rejected (no way to switch off the R2 guard)', async () => {
  const base = toToml(good());
  const variants = [
    'foo = 1\n' + base,
    base + '\n[guard]\nenabled = false\n',
    base.replace('[node]\n', '[node]\nr2_guard = false\n'),
    base.replace('[limits]\n', '[limits]\ndisable_r2_guard = true\n'),
  ];
  for (const v of variants) {
    const r = await validate(v);
    assert.equal(r.code, 1, `accepted a config with an unknown key:\n${v}`);
  }
});

test('repository hygiene', () => {
  const gi = fs.readFileSync(path.join(ROOT, '.gitignore'), 'utf8');
  assert.match(gi, /^\/?target\/?$/m, '.gitignore must ignore target/');
  assert.match(gi, /^\/?\.acc-tmp\/?$/m, '.gitignore must ignore .acc-tmp/');
  for (const f of ['docs/decisions.md', 'docs/open-questions.md', 'CHANGELOG.md', '.cargo/config.toml', 'runner/Cargo.toml']) {
    assert.ok(fs.existsSync(path.join(ROOT, f)), `${f} missing`);
  }
});
