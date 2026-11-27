// csr-07-security, phase 1: node key hygiene (S6), ping helper (S5), decisions.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('acceptance modules are included from the sources', () => {
  assert.match(read('runner', 'src', 'identity.rs'),
    /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-07-security\/identity_tests\.rs"\]\s*mod aiworker_07_identity_tests;/);
  assert.match(read('runner', 'src', 'run.rs'),
    /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-07-security\/run_tests\.rs"\]\s*mod aiworker_07_run_tests;/);
});

test('ConfiguredP2pKey holds no secret', () => {
  const src = read('runner', 'src', 'identity.rs');
  const m = src.match(/pub enum ConfiguredP2pKey\s*\{([\s\S]*?)\n\}/);
  assert.ok(m, 'enum ConfiguredP2pKey not found');
  assert.doesNotMatch(m[1], /\bP2pKey\b/, 'no variant may hold a P2pKey');
  assert.match(m[1], /Known\s*\{/);
  assert.doesNotMatch(src, /fn proven\s*\(/);
  assert.match(src, /fn sign_identity_proof\s*\(/);
});

test('the identity loop signs through sign_identity_proof', () => {
  const o = read('runner', 'src', 'ping', 'outbound.rs');
  assert.match(o, /sign_identity_proof\s*\(/);
  assert.doesNotMatch(o, /\.proven\s*\(/);
});

test('run.rs has the ping helper and the shutdown hook, and uses the helper', () => {
  const r = read('runner', 'src', 'run.rs');
  assert.match(r, /fn ping_listener_wanted\s*\(\s*join:\s*bool,\s*mode:\s*&str\s*\)\s*->\s*bool/);
  assert.ok((r.match(/ping_listener_wanted\s*\(/g) || []).length >= 2, 'definition and use at the bind site');
  assert.match(r, /fn request_shutdown\s*\(\s*\)/);
  assert.match(r, /Notify/);
});

test('the runtime lock is untouched', () => {
  const src = read('runner', 'src', 'network.rs');
  const m = src.match(/fn mainnet_runtime_unlocked\s*\(\s*\)\s*->\s*bool\s*\{([\s\S]{0,200}?)\}/);
  assert.ok(m, 'mainnet_runtime_unlocked() must still exist');
  assert.match(m[1].replace(/\/\/.*$/mg, ''), /^\s*false\s*$/);
});

test('decisions S1-S7 are recorded', () => {
  const d = read('docs', 'decisions.md');
  assert.match(d, /^## csr-07-security \(owner\)/m);
  const sec = d.split(/^## csr-07-security \(owner\)/m)[1].split(/^## /m)[0];
  for (const s of ['S1', 'S2', 'S3', 'S4', 'S5', 'S6', 'S7']) assert.ok(sec.includes(s), s);
  assert.match(sec, /NT SERVICE\\CSR-/);
  assert.match(sec, /SearchWide/);
});
