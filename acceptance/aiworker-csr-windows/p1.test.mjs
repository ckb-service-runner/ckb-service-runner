// csr-windows, phase 1: the runner builds for Windows; Linux behaviour unchanged (W1–W5).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

function rsFiles(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rsFiles(p));
    else if (e.name.endsWith('.rs')) out.push(p);
  }
  return out;
}

test('decisions record the owner section', () => {
  const d = read('docs', 'decisions.md');
  assert.match(d, /^## csr-windows \(owner\)/m);
  for (const w of ['W1', 'W2', 'W3', 'W4', 'W5']) assert.ok(d.includes(w), w);
});

test('default_data_root exists and handles both platforms', () => {
  const src = rsFiles(path.join(ROOT, 'runner', 'src')).map((f) => fs.readFileSync(f, 'utf8')).join('\n');
  assert.match(src, /fn default_data_root\s*\(\s*\)\s*->\s*PathBuf/);
  assert.match(src, /ProgramData/);
  assert.match(src, /\/var\/lib\/csr/);
});

test('unix-only code is gated', () => {
  for (const f of rsFiles(path.join(ROOT, 'runner', 'src'))) {
    const t = fs.readFileSync(f, 'utf8');
    if (/std::os::unix|tokio::signal::unix|PermissionsExt/.test(t)) {
      assert.match(t, /#\[cfg\((unix|not\(windows\)|target_family = "unix")\)\]/, `${path.relative(ROOT, f)} uses Unix APIs without #[cfg(unix)]`);
    }
  }
});

test('the runtime lock is untouched', () => {
  const src = rsFiles(path.join(ROOT, 'runner', 'src')).map((f) => fs.readFileSync(f, 'utf8')).join('\n');
  const m = src.match(/fn mainnet_runtime_unlocked\s*\(\s*\)\s*->\s*bool\s*\{([\s\S]{0,200}?)\}/);
  assert.ok(m, 'mainnet_runtime_unlocked() must still exist');
  assert.match(m[1].replace(/\/\/.*$/mg, ''), /^\s*false\s*$/);
});

test('windows CI workflow builds and uploads csr.exe', () => {
  const w = read('.github', 'workflows', 'windows.yml');
  assert.match(w, /windows-latest/);
  assert.match(w, /cargo build --release -p csr/);
  assert.match(w, /cargo test -p csr/);
  assert.match(w, /csr-windows-x86_64/);
  assert.match(w, /csr\.exe/);
  assert.match(w, /workflow_dispatch/);
});

test('porting notes exist', () => {
  const t = read('docs', 'windows.md');
  assert.match(t, /^## Porting notes/m);
});

test('linux binary still starts with the old default data root', () => {
  const bin = path.join(ROOT, 'target', 'debug', 'csr');
  assert.ok(fs.existsSync(bin), 'run cargo build -p csr first');
  const r = spawnSync(bin, ['--help'], { encoding: 'utf8' });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /data-root/);
});
