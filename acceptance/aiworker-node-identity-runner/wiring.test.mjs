// node-identity-runner: the P2P key is read, checked against the node and used for the proof and add_node.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));

test('identity.rs includes the acceptance test module', () => {
  assert.match(src('identity.rs'), /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-node-identity-runner\/node_identity_tests\.rs"\]/);
});

test('config has node.p2p_key_file', () => {
  assert.match(src('config.rs'), /p2p_key_file/);
  assert.match(read('config', 'csr.example.toml'), /p2p_key_file/);
});

test('the proof is posted and the monitor node is added', () => {
  const all = src('run.rs') + src('ping/outbound.rs') + src('monitor/mod.rs');
  assert.ok(calls(all, 'identity_proof').length >= 1);
  assert.match(all, /\/v1\/identity/);
  assert.match(all, /"add_node"/);
  assert.ok(calls(all, 'parse_monitor_p2p').length >= 1);
});

test('docs explain the key access', () => {
  const t = read('docs', 'install.md');
  assert.match(t, /p2p_key_file/);
  assert.match(t, /secret_key/);
  assert.match(t, /Net/);
  const p = read('spec', 'ping-protocol.md');
  assert.match(p, /CSR-NODE-IDENTITY-v1/);
});
