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

test('keys_for_node offers only the key of the node that serves the URL when there is one', async () => {
  const d = tmp();
  const mk = (name, port) => {
    fs.mkdirSync(path.join(d, name, 'data', 'network'), { recursive: true });
    fs.writeFileSync(path.join(d, name, 'data', 'network', 'secret_key'), 'x');
    if (port) fs.writeFileSync(path.join(d, name, 'ckb.toml'), `[rpc]\nlisten_address = "127.0.0.1:${port}"\n`);
    return path.join(d, name, 'data', 'network', 'secret_key');
  };
  const dev = mk('.devnet', 18114);
  const main = mk('mainnet', 8114);
  const test1 = mk('testnet', 8112);
  const other = mk('noconf', 0);
  const list = `find_keys() { printf '%s\\n' "${dev}" "${main}" "${test1}" "${other}"; }`;
  let r = await sh(`${list}; keys_for_node http://127.0.0.1:8114`);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), [main]);
  r = await sh(`${list}; keys_for_node http://127.0.0.1:9999`);
  assert.deepEqual(r.stdout.trim().split('\n'), [dev, main, test1, other]);
});

test('clear_key_in_config comments out a rejected key', async () => {
  const d = tmp();
  const f = path.join(d, 'c.toml');
  fs.writeFileSync(f, '[node]\nrpc_url = "http://127.0.0.1:8114"\np2p_key_file = "/x/.devnet/data/network/secret_key"\n');
  const r = await sh(`CONF="${f}"; clear_key_in_config`);
  assert.equal(r.code, 0, r.stderr);
  const t = fs.readFileSync(f, 'utf8');
  assert.doesNotMatch(t, /^p2p_key_file/m);
  assert.match(t, /^# p2p_key_file = ""/m);
});

test('a mismatched or unreadable key is cleared and the next one is tried', () => {
  const t = fs.readFileSync(INSTALL, 'utf8');
  const body = t.slice(t.indexOf('setup_identity() {'), t.indexOf('check_runner() {'));
  assert.match(body, /mismatch\)[\s\S]{0,120}clear_key_in_config/);
  assert.match(body, /unreadable\)[\s\S]{0,120}clear_key_in_config/);
  assert.match(body, /keys_for_node "\$RPC"/);
});

test('running_node_dirs reads -C of running ckb processes', async () => {
  const r = await sh(`ps() { printf '%s\\n' '/usr/local/bin/ckb run -C /mnt/ps_data/02_NervosNetwork/BlockChainData' 'ckb run --config-dir /x/testnet' 'vim ckb run -C /nope' '/usr/bin/ckb-light-client run -C /light'; }; running_node_dirs`);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), ['/mnt/ps_data/02_NervosNetwork/BlockChainData', '/x/testnet']);
});

test('find_keys lists the running nodes first, each key once, and searches each disk under /mnt', async () => {
  const d = tmp();
  const key = path.join(d, 'node', 'data', 'network', 'secret_key');
  fs.mkdirSync(path.dirname(key), { recursive: true });
  fs.writeFileSync(key, 'x');
  const r = await sh(`running_node_dirs() { echo "${path.join(d, 'node')}"; echo /does/not/exist; }; find() { echo "${key}"; echo /other/data/network/secret_key; }; find_keys`);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), [key, '/other/data/network/secret_key']);
  assert.match(fs.readFileSync(INSTALL, 'utf8'), /\/mnt\/\* \/media\/\*/);
});
