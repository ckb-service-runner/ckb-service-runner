// csr-windows, phase 3: Windows package, docs and release notes (W1–W5).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

function listZip(zip) {
  const r = spawnSync('python3', ['-c', 'import sys,zipfile; print("\\n".join(zipfile.ZipFile(sys.argv[1]).namelist()))', zip], { encoding: 'utf8' });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout.trim().split('\n');
}

test('package-windows.sh builds a clean zip', () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), 'csr-win-'));
  const exe = path.join(d, 'csr.exe');
  fs.writeFileSync(exe, 'MZ fake');
  const ver = '0.0.0-test';
  const r = spawnSync('bash', ['scripts/package-windows.sh', ver, exe], { encoding: 'utf8', cwd: ROOT });
  assert.equal(r.status, 0, r.stderr + r.stdout);
  const zip = path.join(ROOT, 'dist', `csr-${ver}-x86_64-windows.zip`);
  assert.ok(fs.existsSync(zip), zip);
  const names = listZip(zip);
  const has = (re) => names.some((n) => re.test(n));
  for (const re of [/csr\.exe$/, /csr\.example\.toml$/, /install\.ps1$/, /uninstall\.ps1$/, /README-windows\.txt$/, /networks\//]) {
    assert.ok(has(re), String(re));
  }
  if (fs.existsSync(path.join(ROOT, 'LICENSE'))) assert.ok(has(/LICENSE/), 'LICENSE');
  for (const n of names) {
    assert.doesNotMatch(n, /secret_key|\.key$|\.pem$/, n);
    if (n.endsWith('.toml')) assert.ok(/csr\.example\.toml$|networks\/[^/]+\.toml$/.test(n), `unexpected toml ${n}`);
  }
  fs.rmSync(zip);
});

test('release.sh includes the windows zip in SHA256SUMS when present', () => {
  const t = read('scripts', 'release.sh');
  assert.match(t, /x86_64-windows\.zip/);
  assert.match(t, /SHA256SUMS/);
});

test('docs/windows.md covers install, Neuron, firewall, update, uninstall', () => {
  const t = read('docs', 'windows.md');
  for (const s of ['install.ps1', 'uninstall.ps1', 'Administrator', 'Get-FileHash', 'SHA256SUMS', '8130', '8140',
    'Net', 'Neuron', 'light client', '-Rpc', '-CkbDir', '-NoRanking', 'beta']) {
    assert.ok(t.includes(s), s);
  }
  assert.match(t, /SmartScreen|unsigned/i);
  assert.match(t, /sleep/i);
  assert.match(t, /^## Porting notes/m);
});

test('README and quick start point to the Windows guide', () => {
  assert.match(read('README.md'), /Windows \(beta\)[\s\S]{0,400}docs\/windows\.md/);
  assert.match(read('docs', 'quickstart.md'), /docs\/windows\.md|windows\.md/);
});

test('release notes and open questions', () => {
  const r = read('docs', 'release.md');
  assert.match(r, /windows\.yml/);
  assert.match(r, /package-windows\.sh/);
  assert.match(read('docs', 'open-questions.md'), /Windows[\s\S]{0,300}Neuron/);
});
