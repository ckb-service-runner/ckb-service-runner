// installer fixes (first real run, 31 Oct 2026): nodes from existing CSR configs; refuse a release without templates.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { execFile } from 'node:child_process';

const ROOT = process.cwd();
const INSTALL = path.join(ROOT, 'scripts', 'install.sh');
const servers = [];
after(() => servers.forEach((s) => s.close()));
const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'csr-fix-'));

function sh(snippet) {
  const script = `set -euo pipefail\nsource "${INSTALL}"\n${snippet}`;
  return new Promise((resolve) => execFile('bash', ['-c', script], { timeout: 60000 },
    (err, stdout, stderr) => resolve({ code: err ? (err.code ?? 1) : 0, stdout, stderr })));
}
function fakeNode(chain) {
  return new Promise((resolve) => {
    const s = http.createServer((req, res) => {
      let b = '';
      req.on('data', (d) => { b += d; });
      req.on('end', () => {
        const m = JSON.parse(b || '{}');
        res.end(JSON.stringify({ id: m.id, jsonrpc: '2.0', result: m.method === 'get_blockchain_info' ? { chain } : {} }));
      });
    });
    s.listen(0, '127.0.0.1', () => { servers.push(s); resolve(`http://127.0.0.1:${s.address().port}`); });
  });
}
const closed = () => new Promise((r) => { const s = http.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => r(`http://127.0.0.1:${p}`)); }); });

test('configured_rpc_urls reads csr.toml and csr-<network>.toml, loopback only', async () => {
  const d = tmp();
  fs.writeFileSync(path.join(d, 'csr.toml'), 'network = "testnet"\n[node]\nrpc_url = "http://127.0.0.1:8112"\n');
  fs.writeFileSync(path.join(d, 'csr-mainnet.toml'), 'network = "mainnet"\n[node]\nrpc_url = "http://127.0.0.1:8114"   # main\n');
  fs.writeFileSync(path.join(d, 'csr-x.toml'), '[node]\nrpc_url = "http://192.168.1.5:8114"\n');
  const r = await sh(`DATA="${d}"; configured_rpc_urls | sort`);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), ['http://127.0.0.1:8112', 'http://127.0.0.1:8114']);
  const e = await sh(`DATA="${tmp()}"; configured_rpc_urls; echo end`);
  assert.equal(e.stdout.trim(), 'end');
});

test('find_nodes finds the node of an existing csr.toml without --rpc', async () => {
  const d = tmp();
  const testnet = await fakeNode('ckb_testnet');
  const mainnet = await fakeNode('ckb');
  fs.writeFileSync(path.join(d, 'csr.toml'), `network = "testnet"\n[node]\nrpc_url = "${testnet}"\n`);
  const r = await sh(`DATA="${d}"; RPCS=(); DEFAULT_RPCS=("${mainnet}"); find_keys() { :; }; find_nodes`);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), [`testnet ${testnet}`, `mainnet ${mainnet}`]);
});

test('release_has_templates', async () => {
  const d = tmp();
  let r = await sh(`release_has_templates "${d}" && echo yes || echo no`);
  assert.equal(r.stdout.trim(), 'no');
  for (const f of ['csr@.service', 'csr-update@.path', 'csr-update@.service']) fs.writeFileSync(path.join(d, f), '');
  r = await sh(`release_has_templates "${d}" && echo yes || echo no`);
  assert.equal(r.stdout.trim(), 'yes');
});

test('install_binary checks the release before installing anything', () => {
  const t = fs.readFileSync(INSTALL, 'utf8');
  const body = t.slice(t.indexOf('install_binary() {'));
  const check = body.indexOf('release_has_templates "$dir"');
  const firstInstall = body.indexOf('install -m');
  assert.ok(check > 0 && check < firstInstall, 'the check must come before the first install command');
  assert.match(body.slice(check, check + 300), /die/);
});
