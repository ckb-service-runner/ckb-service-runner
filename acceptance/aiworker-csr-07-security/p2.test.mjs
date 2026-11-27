// csr-07-security, phase 2: the runner can run as a Windows service (S1). Linux behaviour unchanged.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

// Lines of a TOML section: from its header up to the next line that starts a section.
function tomlSection(text, header) {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((l) => l.trim().replace(/"/g, "'") === header);
  if (start < 0) return null;
  const out = [];
  for (let i = start + 1; i < lines.length; i++) {
    if (/^\s*\[/.test(lines[i])) break;
    out.push(lines[i]);
  }
  return out.join('\n');
}

test('windows-service is a Windows-only dependency', () => {
  const sec = tomlSection(read('runner', 'Cargo.toml'), "[target.'cfg(windows)'.dependencies]");
  assert.ok(sec, "section [target.'cfg(windows)'.dependencies] not found");
  assert.match(sec, /^\s*windows-service\s*=/m);
  const top = read('runner', 'Cargo.toml').split(/^\[target/m)[0];
  assert.doesNotMatch(top, /windows-service/, 'must not be a dependency on other platforms');
  assert.match(read('Cargo.lock'), /name = "windows-service"/);
});

test('winservice.rs follows the service protocol', () => {
  const w = read('runner', 'src', 'winservice.rs');
  assert.match(w, /cfg\(windows\)/);
  assert.match(w, /service_dispatcher/);
  assert.match(w, /service_control_handler/);
  assert.match(w, /ServiceControl::Stop/);
  assert.match(w, /ServiceControl::Shutdown/);
  assert.match(w, /ServiceState::StopPending/);
  assert.match(w, /ServiceState::Running/);
  assert.match(w, /ServiceState::Stopped/);
  assert.match(w, /request_shutdown\s*\(/);
  assert.match(w, /ServiceSpecific\s*\(\s*1\s*\)/);
  assert.match(read('runner', 'src', 'main.rs'), /^mod winservice;/m);
});

test('windows.yml also runs on pull requests', () => {
  const w = read('.github', 'workflows', 'windows.yml');
  assert.match(w, /pull_request/);
  assert.match(w, /workflow_dispatch/);
  assert.match(w, /csr-windows-x86_64/);
  assert.match(w, /cargo test -p csr --bins/);
});

test('on Linux the hidden subcommand refuses and stays out of --help', () => {
  const b = spawnSync('bash', ['acceptance/a1/cargo.sh', 'build', '-q', '-p', 'csr'], { encoding: 'utf8', cwd: ROOT, maxBuffer: 64 * 1024 * 1024 });
  assert.equal(b.status, 0, b.stderr);
  const bin = path.join(ROOT, 'target', 'debug', 'csr');
  const r = spawnSync(bin, ['windows-service'], { encoding: 'utf8' });
  assert.equal(r.status, 2, `exit code ${r.status}: ${r.stdout}${r.stderr}`);
  assert.match(r.stdout + r.stderr, /only available on Windows/);
  const h = spawnSync(bin, ['--help'], { encoding: 'utf8' });
  assert.equal(h.status, 0);
  assert.doesNotMatch(h.stdout, /windows-service/);
  const run = spawnSync(bin, ['run', '--help'], { encoding: 'utf8' });
  assert.equal(run.status, 0, 'csr run still exists');
});
