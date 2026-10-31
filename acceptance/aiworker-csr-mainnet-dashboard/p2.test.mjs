// csr-mainnet-dashboard, phase 2: installer finds every local node and installs one instance per network
// (D31, D32). The installer is sourced in bash with stubs; no root, no systemd, no network.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { execFile } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const INSTALL = path.join(ROOT, 'scripts', 'install.sh');
const EXAMPLE = path.join(ROOT, 'config', 'csr.example.toml');

const servers = [];
after(() => servers.forEach((s) => s.close()));

function tmp() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'csr-p2-'));
}

// Runs a bash snippet after sourcing install.sh. Async, so the fake nodes of this process can answer.
function sh(snippet, env = {}) {
  const script = `set -euo pipefail
source "${INSTALL}"
own_csr() { :; }
systemctl() { echo "systemctl $*" >> "$LOG"; }
${snippet}`;
  return new Promise((resolve) => {
    execFile('bash', ['-c', script], { env: { ...process.env, ...env }, timeout: 60000 }, (err, stdout, stderr) => {
      resolve({ code: err ? (err.code ?? 1) : 0, stdout, stderr });
    });
  });
}

// A fake CKB node: answers get_blockchain_info with the given chain, everything else with a result.
function fakeNode(chain) {
  return new Promise((resolve) => {
    const s = http.createServer((req, res) => {
      let body = '';
      req.on('data', (d) => { body += d; });
      req.on('end', () => {
        const m = JSON.parse(body || '{}');
        const result = m.method === 'get_blockchain_info'
          ? { chain, is_initial_block_download: false }
          : m.method === 'get_tip_block_number' ? '0x10' : {};
        res.setHeader('content-type', 'application/json');
        res.end(JSON.stringify({ id: m.id, jsonrpc: '2.0', result }));
      });
    });
    s.listen(0, '127.0.0.1', () => { servers.push(s); resolve(`http://127.0.0.1:${s.address().port}`); });
  });
}

function closedPort() {
  return new Promise((resolve) => {
    const s = http.createServer();
    s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => resolve(`http://127.0.0.1:${p}`)); });
  });
}

test('installer is no longer testnet-only and stays sourceable', () => {
  const t = read('scripts', 'install.sh');
  assert.doesNotMatch(t.split('\n').slice(0, 3).join('\n'), /Testnet only/i);
  assert.doesNotMatch(t, /The installer sets up testnet only/);
  assert.match(t, /if \[ "\$\{BASH_SOURCE\[0\]\}" = "\$0" \]; then main "\$@"; fi/);
});

test('chain_network and instance_ports', async () => {
  const r = await sh(`chain_network ckb; chain_network ckb_testnet; chain_network ckb_dev || echo none
instance_ports testnet; instance_ports mainnet`);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), ['mainnet', 'testnet', 'none', '8130 8131', '8140 8141']);
});

test('rpc_url_from_ckb_toml reads [rpc] listen_address', async () => {
  const d = tmp();
  const w = (name, body) => { fs.writeFileSync(path.join(d, name), body); return path.join(d, name); };
  const a = w('a.toml', '[network]\nlisten_addresses = ["/ip4/0.0.0.0/tcp/8115"]\n\n[rpc]\nlisten_address = "127.0.0.1:8112"\nmodules = ["Net"]\n');
  const b = w('b.toml', '[rpc]\n# listen_address = "127.0.0.1:9999"\nlisten_address = "0.0.0.0:8124" # all\n');
  const c = w('c.toml', '[rpc]\nlisten_address = "192.168.1.5:8114"\n');
  const e = w('e.toml', '[db]\nlisten_address = "127.0.0.1:7777"\n[rpc]\nmodules = []\n');
  const r = await sh(`rpc_url_from_ckb_toml "${a}"; rpc_url_from_ckb_toml "${b}"
rpc_url_from_ckb_toml "${c}" || echo none-c; rpc_url_from_ckb_toml "${e}" || echo none-e`);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'),
    ['http://127.0.0.1:8112', 'http://127.0.0.1:8124', 'none-c', 'none-e']);
});

test('find_nodes with --rpc URLs: one line per network, testnet first, first URL wins', async () => {
  const main1 = await fakeNode('ckb');
  const test1 = await fakeNode('ckb_testnet');
  const main2 = await fakeNode('ckb');
  const dev = await fakeNode('ckb_dev');
  const dead = await closedPort();
  const r = await sh(`RPCS=("${dead}" "${main1}" "${dev}" "${main2}" "${test1}"); find_nodes`, { LOG: '/dev/null' });
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), [`testnet ${test1}`, `mainnet ${main1}`]);
});

test('find_nodes without --rpc reads the ckb.toml next to each found key', async () => {
  const d = tmp();
  const node = await fakeNode('ckb');
  const port = node.split(':').pop();
  fs.mkdirSync(path.join(d, 'ckb-main', 'data', 'network'), { recursive: true });
  fs.writeFileSync(path.join(d, 'ckb-main', 'data', 'network', 'secret_key'), 'x');
  fs.writeFileSync(path.join(d, 'ckb-main', 'ckb.toml'), `[rpc]\nlisten_address = "127.0.0.1:${port}"\n`);
  const dead = await closedPort();
  const r = await sh(`RPCS=(); DEFAULT_RPCS=("${dead}")
find_keys() { echo "${d}/ckb-main/data/network/secret_key"; }
find_nodes`, { LOG: '/dev/null' });
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(r.stdout.trim().split('\n'), [`mainnet ${node}`]);
});

test('find_nodes prints nothing and fails when no node answers', async () => {
  const dead = await closedPort();
  const r = await sh(`RPCS=("${dead}"); find_nodes`, { LOG: '/dev/null' });
  assert.notEqual(r.code, 0);
  assert.equal(r.stdout.trim(), '');
});

test('write_instance_config writes one config per network with its own ports', async () => {
  const d = tmp();
  const r = await sh(`DATA="${d}"; EXAMPLE="${EXAMPLE}"; PING_MODE=outbound; RUNTIME=true
write_instance_config testnet http://127.0.0.1:8114
write_instance_config mainnet http://127.0.0.1:8112`, { LOG: '/dev/null' });
  assert.equal(r.code, 0, r.stderr);
  const t = fs.readFileSync(path.join(d, 'csr-testnet.toml'), 'utf8');
  const m = fs.readFileSync(path.join(d, 'csr-mainnet.toml'), 'utf8');
  assert.match(t, /^network = "testnet"/m);
  assert.match(m, /^network = "mainnet"/m);
  assert.match(t, /^rpc_url = "http:\/\/127\.0\.0\.1:8114"/m);
  assert.match(m, /^rpc_url = "http:\/\/127\.0\.0\.1:8112"/m);
  assert.match(t, /^bind = "127\.0\.0\.1:8130"/m);
  assert.match(m, /^bind = "127\.0\.0\.1:8140"/m);
  assert.match(t, /^ping_bind = "0\.0\.0\.0:8131"/m);
  assert.match(m, /^ping_bind = "0\.0\.0\.0:8141"/m);
  assert.match(m, /^ping_mode = "outbound"/m);
  assert.ok(!fs.existsSync(path.join(d, 'csr.toml')));
});

test('write_instance_config keeps an existing config and only updates rpc_url', async () => {
  const d = tmp();
  const f = path.join(d, 'csr-testnet.toml');
  fs.writeFileSync(f, 'network = "testnet"\n\n[node]\nrpc_url = "http://127.0.0.1:8114"\n\n[dashboard]\nbind = "127.0.0.1:9000"\n');
  const r = await sh(`DATA="${d}"; EXAMPLE="${EXAMPLE}"; PING_MODE=outbound; RUNTIME=true
write_instance_config testnet http://127.0.0.1:8124`, { LOG: '/dev/null' });
  assert.equal(r.code, 0, r.stderr);
  const t = fs.readFileSync(f, 'utf8');
  assert.match(t, /^rpc_url = "http:\/\/127\.0\.0\.1:8124"/m);
  assert.match(t, /^bind = "127\.0\.0\.1:9000"/m);
});

test('set_peer_url links the two dashboards', async () => {
  const d = tmp();
  const r = await sh(`DATA="${d}"; EXAMPLE="${EXAMPLE}"; PING_MODE=outbound; RUNTIME=true
write_instance_config testnet http://127.0.0.1:8114
write_instance_config mainnet http://127.0.0.1:8112
set_peer_url testnet http://127.0.0.1:8140
set_peer_url mainnet http://127.0.0.1:8130
printf 'network = "testnet"\\n[dashboard]\\nbind = "127.0.0.1:8130"\\n' > "${d}/old.toml"
cp "${d}/old.toml" "${d}/csr-devnet.toml"; set_peer_url devnet http://x:1`, { LOG: '/dev/null' });
  assert.equal(r.code, 0, r.stderr);
  const dash = (f) => fs.readFileSync(path.join(d, f), 'utf8').split('[dashboard]')[1].split('\n[')[0];
  assert.match(dash('csr-testnet.toml'), /^peer_url = "http:\/\/127\.0\.0\.1:8140"/m);
  assert.match(dash('csr-mainnet.toml'), /^peer_url = "http:\/\/127\.0\.0\.1:8130"/m);
  assert.match(dash('csr-devnet.toml'), /^peer_url = "http:\/\/x:1"/m);          // key added when missing
  assert.equal((dash('csr-testnet.toml').match(/^peer_url/mg) || []).length, 1);
});

test('migrate_legacy moves csr.toml to csr-<network>.toml and retires csr.service', async () => {
  const d = tmp();
  const log = path.join(d, 'log');
  fs.writeFileSync(path.join(d, 'csr.toml'), 'network = "testnet"\n[dashboard]\nbind = "127.0.0.1:8130"\n');
  const r = await sh(`DATA="${d}"; migrate_legacy`, { LOG: log });
  assert.equal(r.code, 0, r.stderr);
  assert.ok(!fs.existsSync(path.join(d, 'csr.toml')));
  assert.match(fs.readFileSync(path.join(d, 'csr-testnet.toml'), 'utf8'), /bind = "127\.0\.0\.1:8130"/);
  const l = fs.readFileSync(log, 'utf8');
  assert.match(l, /disable --now .*csr\.service/);
  assert.match(l, /csr-update\.path/);
});

test('migrate_legacy does nothing without csr.toml and never overwrites', async () => {
  const d = tmp();
  const log = path.join(d, 'log');
  let r = await sh(`DATA="${d}"; migrate_legacy`, { LOG: log });
  assert.equal(r.code, 0, r.stderr);
  assert.ok(!fs.existsSync(log));
  fs.writeFileSync(path.join(d, 'csr.toml'), 'network = "mainnet"\n');
  fs.writeFileSync(path.join(d, 'csr-mainnet.toml'), 'network = "mainnet"\n# keep me\n');
  r = await sh(`DATA="${d}"; migrate_legacy`, { LOG: log });
  assert.equal(r.code, 0, r.stderr);
  assert.match(fs.readFileSync(path.join(d, 'csr-mainnet.toml'), 'utf8'), /keep me/);
  assert.ok(fs.existsSync(path.join(d, 'csr.toml')));
});

test('systemd template units, update helper and package', () => {
  const svc = read('config', 'csr@.service');
  assert.match(svc, /^ExecStart=\/usr\/local\/bin\/csr --data-root \/var\/lib\/csr --config \/var\/lib\/csr\/csr-%i\.toml run$/m);
  for (const k of ['User=csr', 'ProtectSystem=strict', 'ProtectHome=yes', 'ReadWritePaths=/var/lib/csr', 'MemoryMax=256M', 'Nice=19']) {
    assert.ok(svc.includes(k), k);
  }
  const p = read('config', 'csr-update@.path');
  assert.match(p, /^PathExists=\/var\/lib\/csr\/%i\/update\/ready\.json$/m);
  assert.match(p, /^Unit=csr-update@%i\.service$/m);
  assert.match(read('config', 'csr-update@.service'), /^ExecStart=\/usr\/local\/lib\/csr\/install-update\.sh \/var\/lib\/csr\/%i\/update$/m);
  const u = read('scripts', 'install-update.sh');
  assert.match(u, /csr@\*\.service/);
  assert.doesNotMatch(u, /^systemctl restart csr$/m);
  const pkg = read('scripts', 'package.sh');
  for (const f of ['config/csr@.service', 'config/csr-update@.path', 'config/csr-update@.service']) assert.ok(pkg.includes(f), f);
});

test('installer enables per-network units and confirms mainnet', () => {
  const t = read('scripts', 'install.sh');
  assert.match(t, /^DEFAULT_RPCS=\("http:\/\/127\.0\.0\.1:8114"\)$/m);
  assert.match(t, /csr@\$\{?net\}?\.service|csr@"\$net"\.service|csr@\$NET\.service|csr@\$\{NET\}\.service/i);
  assert.match(t, /csr-update@/);
  assert.match(t, /confirm-mainnet/);
  assert.match(t, /--config "\$CONF"/);
  assert.match(t, /--rpc\)[^\n]*RPCS\+=/);
});
