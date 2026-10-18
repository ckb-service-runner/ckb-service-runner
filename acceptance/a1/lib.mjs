// Shared helpers for the CSR acceptance tests (written by the spec author).
import { spawn } from 'node:child_process';
import http from 'node:http';
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

export const ROOT = process.cwd();
export const CSR = path.join(ROOT, 'target', 'debug', 'csr');
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const procs = new Set();
const closers = new Set();

export function onCleanup(fn) { closers.add(fn); }

export async function cleanup() {
  for (const p of procs) { try { p.kill('SIGKILL'); } catch { /* ignore */ } }
  procs.clear();
  for (const c of closers) { try { await c(); } catch { /* ignore */ } }
  closers.clear();
}
process.on('exit', () => {
  for (const p of procs) { try { p.kill('SIGKILL'); } catch { /* ignore */ } }
});

export function tmpDir(name) {
  const base = path.join(ROOT, '.acc-tmp');
  fs.mkdirSync(base, { recursive: true });
  return fs.mkdtempSync(path.join(base, `${name}-`));
}

export function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.on('error', reject);
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

export const hex = (n) => '0x' + BigInt(n).toString(16);
export const blockHash = (n) => '0x' + crypto.createHash('sha256').update(`mock-block-${n}`).digest('hex');
const zero32 = '0x' + '00'.repeat(32);

export function mockHeader(n) {
  const num = BigInt(n);
  const epoch = (num / 1800n) | ((num % 1800n) << 24n) | (1800n << 40n);
  return {
    version: '0x0',
    compact_target: '0x1e015555',
    timestamp: hex(1700000000000 + Number(n) * 8000),
    number: hex(n),
    epoch: hex(epoch),
    parent_hash: Number(n) > 0 ? blockHash(Number(n) - 1) : zero32,
    transactions_root: '0x' + '11'.repeat(32),
    proposals_hash: zero32,
    extra_hash: zero32,
    dao: '0x' + '22'.repeat(32),
    nonce: '0x0',
    hash: blockHash(n),
  };
}

// In-process mock of a CKB full node JSON-RPC endpoint. Change behaviour with mock.set({...}).
export async function startMockNode(initial = {}) {
  const state = {
    chain: 'ckb_testnet',
    tip: 1000,
    bestKnown: 1000,
    ibd: false,
    indexer: true,
    indexerTip: 1000,
    light: false,
    nodeId: 'QmMockNodeIdAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
    version: '0.210.0 (mock)',
    peers: [{ is_outbound: true }, { is_outbound: false }],
    ...initial,
  };
  const counter = { calls: 0 };
  const notFound = (id, method) => ({ jsonrpc: '2.0', id, error: { code: -32601, message: `Method not found: ${method}` } });

  function call(req) {
    counter.calls += 1;
    const id = req && req.id !== undefined ? req.id : null;
    const method = req && req.method;
    const params = (req && req.params) || [];
    const ok = (result) => ({ jsonrpc: '2.0', id, result });
    const s = state;
    switch (method) {
      case 'get_blockchain_info':
        if (s.light) return notFound(id, method);
        return ok({
          chain: s.chain, is_initial_block_download: s.ibd, epoch: mockHeader(s.tip).epoch,
          median_time: hex(1700000000000 + s.tip * 8000), difficulty: '0x1', alerts: [],
        });
      case 'sync_state':
        if (s.light) return notFound(id, method);
        return ok({
          ibd: s.ibd, assume_valid_target_reached: true, assume_valid_target: zero32,
          min_chain_work_reached: true, min_chain_work: '0x0',
          best_known_block_number: hex(s.bestKnown),
          best_known_block_timestamp: hex(1700000000000 + s.bestKnown * 8000),
          orphan_blocks_count: '0x0', orphan_blocks_size: '0x0', inflight_blocks_count: '0x0',
          unverified_tip_number: hex(s.tip), unverified_tip_hash: blockHash(s.tip),
          tip_number: hex(s.tip), tip_hash: blockHash(s.tip),
          fast_time: '0x3e8', normal_time: '0x4e2', low_time: '0x5dc',
        });
      case 'get_tip_header':
        return ok(mockHeader(s.tip));
      case 'get_tip_block_number':
        return ok(hex(s.tip));
      case 'get_header_by_number': {
        const n = Number(BigInt(params[0]));
        return ok(n <= s.tip ? mockHeader(n) : null);
      }
      case 'get_block_hash': {
        const n = Number(BigInt(params[0]));
        return ok(n <= s.tip ? blockHash(n) : null);
      }
      case 'local_node_info':
        return ok({
          version: s.version, node_id: s.nodeId, active: true, addresses: [], protocols: [],
          connections: hex(s.peers.length),
        });
      case 'get_peers':
        return ok(s.peers.map((p, i) => ({
          version: '0.210.0',
          node_id: p.node_id || `QmPeer${i}`,
          addresses: [{ address: `/ip4/10.0.0.${i + 1}/tcp/8115/p2p/QmPeer${i}`, score: '0x1' }],
          is_outbound: !!p.is_outbound,
          connected_duration: '0x3e8',
          last_ping_duration: '0x10',
          sync_state: {
            best_known_header_hash: null, best_known_header_number: null,
            last_common_header_hash: null, last_common_header_number: null,
            unknown_header_list_size: '0x0', inflight_count: '0x0', can_fetch_count: '0x0',
          },
          protocols: [],
        })));
      case 'get_indexer_tip':
        if (!s.indexer) return notFound(id, method);
        return ok({ block_number: hex(s.indexerTip), block_hash: blockHash(s.indexerTip) });
      case 'tx_pool_info':
        return ok({
          tip_hash: blockHash(s.tip), tip_number: hex(s.tip), pending: '0x2', proposed: '0x1', orphan: '0x0',
          total_tx_size: '0x0', total_tx_cycles: '0x0', min_fee_rate: '0x3e8', min_rbf_rate: '0x5dc',
          last_txs_updated_at: '0x0', tx_size_limit: '0x7d000', max_tx_pool_size: '0xaba9500',
          verify_queue_size: '0x0',
        });
      default:
        return notFound(id, method);
    }
  }

  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      let parsed;
      try { parsed = JSON.parse(body); } catch { res.statusCode = 400; res.end(); return; }
      const out = Array.isArray(parsed) ? parsed.map(call) : call(parsed);
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify(out));
    });
  });
  const port = await freePort();
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  let closed = false;
  const close = () => new Promise((r) => {
    if (closed) { r(); return; }
    closed = true;
    server.closeAllConnections();
    server.close(() => r());
  });
  onCleanup(close);
  return {
    port, url: `http://127.0.0.1:${port}`, state,
    set: (patch) => Object.assign(state, patch),
    get calls() { return counter.calls; },
    close,
  };
}

const FLOAT_KEYS = new Set(['min_ckb_per_mcycle']);
function tomlValue(k, v) {
  if (typeof v === 'string') return JSON.stringify(v);
  if (typeof v === 'boolean') return String(v);
  if (typeof v === 'number') return FLOAT_KEYS.has(k) && Number.isInteger(v) ? v.toFixed(1) : String(v);
  if (Array.isArray(v)) return '[' + v.map((x) => tomlValue(k, x)).join(', ') + ']';
  throw new Error(`unsupported TOML value for ${k}`);
}
export function toToml(obj) {
  let top = '';
  let tables = '';
  for (const [k, v] of Object.entries(obj)) {
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      tables += `\n[${k}]\n`;
      for (const [k2, v2] of Object.entries(v)) tables += `${k2} = ${tomlValue(k2, v2)}\n`;
    } else {
      top += `${k} = ${tomlValue(k, v)}\n`;
    }
  }
  return top + tables;
}

export function baseConfig({ rpcPort, dashPort, pingPort }) {
  return {
    network: 'testnet',
    node: { rpc_url: `http://127.0.0.1:${rpcPort}`, node_type: 'auto' },
    dashboard: { bind: `127.0.0.1:${dashPort}`, password_hash: '' },
    ranking: { join: false, ping_mode: 'inbound', ping_bind: `127.0.0.1:${pingPort}` },
    monitor: { url: '', pubkey: '' },
    runtime: { enabled: false, allowed_services: [], min_ckb_per_mcycle: 0 },
    limits: {
      max_concurrent_jobs: 2, max_cycles_per_job: 70000000, max_vm_memory_mb: 4,
      max_job_wall_seconds: 30, pause_above_cpu_percent: 85, min_free_ram_mb: 64, min_free_disk_gb: 1,
    },
    storage: { path: 'service_data', total_quota_mb: 256, per_service_quota_mb: 64 },
    logging: { level: 'info' },
  };
}

export function writeConfig(dir, cfg, name = 'csr.toml') {
  const f = path.join(dir, name);
  fs.writeFileSync(f, toToml(cfg));
  return f;
}

// Creates a data root, a mock node and a config. cfg(c) may mutate and must return the config.
export async function setup(name, { node = {}, cfg = (c) => c } = {}) {
  const dir = tmpDir(name);
  const mock = await startMockNode(node);
  const dashPort = await freePort();
  const pingPort = await freePort();
  const config = cfg(baseConfig({ rpcPort: mock.port, dashPort, pingPort }));
  const configPath = writeConfig(dir, config);
  return {
    dir, mock, dashPort, pingPort, config, configPath,
    args: ['--data-root', dir, '--config', configPath],
    base: `http://127.0.0.1:${dashPort}`,
    ping: `http://127.0.0.1:${pingPort}`,
  };
}

export function runCsr(args, { env = {}, input, timeoutMs = 60000 } = {}) {
  return new Promise((resolve) => {
    const p = spawn(CSR, args, { env: { ...process.env, ...env }, stdio: ['pipe', 'pipe', 'pipe'] });
    procs.add(p);
    let stdout = '';
    let stderr = '';
    let done = false;
    const t = setTimeout(() => p.kill('SIGKILL'), timeoutMs);
    const finish = (r) => {
      if (done) return;
      done = true;
      clearTimeout(t);
      procs.delete(p);
      resolve(r);
    };
    p.stdout.on('data', (d) => { stdout += d; });
    p.stderr.on('data', (d) => { stderr += d; });
    p.on('error', (e) => finish({ code: -1, signal: null, stdout, stderr: stderr + String(e) }));
    p.on('close', (code, signal) => finish({ code, signal, stdout, stderr }));
    p.stdin.on('error', () => {});
    p.stdin.end(input === undefined ? '' : input);
  });
}

// Starts `csr ... run` (fast tick on by default) and waits until healthUrl answers 200.
export async function startCsr(args, { env = {}, healthUrl, timeoutMs = 30000 } = {}) {
  const started = Date.now();
  const p = spawn(CSR, args, {
    env: { ...process.env, CSR_TEST_FAST_TICK: '1', ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  procs.add(p);
  const out = { stdout: '', stderr: '' };
  p.stdout.on('data', (d) => { out.stdout += d; });
  p.stderr.on('data', (d) => { out.stderr += d; });
  p.on('error', (e) => { out.stderr += String(e); });
  let exit = null;
  const exited = new Promise((r) => p.on('exit', (code, signal) => {
    exit = { code, signal };
    procs.delete(p);
    r(exit);
  }));
  for (;;) {
    if (exit) throw new Error(`csr exited early (${JSON.stringify(exit)}):\n${out.stderr.slice(-3000)}`);
    if (Date.now() - started > timeoutMs) throw new Error(`csr not healthy after ${timeoutMs} ms:\n${out.stderr.slice(-3000)}`);
    try {
      const r = await fetch(healthUrl);
      if (r.ok) break;
    } catch { /* not up yet */ }
    await sleep(150);
  }
  const startupMs = Date.now() - started;
  async function stop() {
    if (exit) return exit;
    p.kill('SIGTERM');
    const t = setTimeout(() => p.kill('SIGKILL'), 8000);
    const e = await exited;
    clearTimeout(t);
    return e;
  }
  return { proc: p, out, startupMs, stop, exited };
}

export async function getJson(url, headers = {}) {
  const r = await fetch(url, { headers });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not JSON */ }
  return { status: r.status, json, text, headers: r.headers };
}

export async function waitFor(fn, timeoutMs = 8000, stepMs = 200) {
  const end = Date.now() + timeoutMs;
  let last;
  while (Date.now() < end) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) { last = e; }
    await sleep(stepMs);
  }
  throw new Error(`condition not met within ${timeoutMs} ms: ${last instanceof Error ? last.message : JSON.stringify(last)}`);
}
