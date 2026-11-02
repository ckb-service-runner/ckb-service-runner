// Windows installer usability (after the first real installs, Nov 2026): light-client message, node folder from
// the running ckb.exe, Neuron light keys ignored, double-click launchers, Start menu links, browser, package.
// Content checks only; nothing is executed on Windows here.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('light client: recognised via get_tip_header, port 9000 probed, own message', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /get_tip_header/);
  assert.match(t, /127\.0\.0\.1:9000/);
  assert.match(t, /LightClients/);
  assert.match(t, /a CKB light client answers at/);
});

test('node folder from the running ckb.exe; Neuron light keys never used', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /function\s+Get-RunningNodeDirs\b/);
  assert.match(t, /Win32_Process/);
  assert.match(t, /ckb\.exe/);
  assert.match(t, /function\s+Get-CkbConfigDirFromCommandLine\b/);
  assert.match(t, /--config-dir/);
  assert.match(t, /function\s+Test-LightClientPath\b/);
  assert.ok(t.includes('chains[\\\\/]light'), 'light-client path test');
  // the running folders feed the key search, the ckb.toml lookup and the node candidates
  assert.ok((t.match(/\$script:RunningNodeDirs/g) || []).length >= 4);
});

test('-C parsing regex matches the forms ckb and Neuron use', () => {
  const t = read('scripts', 'install.ps1');
  const m = t.match(/\[regex\]::Match\(\$CommandLine, '([^']+)'\)/);
  assert.ok(m, 'regex found');
  const re = new RegExp(m[1].replace(/\\\\/g, '\\'));
  const dir = (s) => { const r = re.exec(s); return r && (r[1] || r[2]); };
  assert.equal(dir('"C:\\Neuron\\bin\\ckb.exe" run -C "C:\\02_CKB_Blockchain" --indexer'), 'C:\\02_CKB_Blockchain');
  assert.equal(dir('ckb.exe run -C C:\\ckb-testnet'), 'C:\\ckb-testnet');
  assert.equal(dir('ckb.exe run --config-dir=D:\\My Node'), 'D:\\My');
  assert.equal(dir('ckb.exe run --config-dir "D:\\My Node"'), 'D:\\My Node');
  assert.equal(dir('ckb.exe run'), null);
});

test('installer: unblock, Start menu link, browser, -NoBrowser', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /Unblock-File/);
  assert.match(t, /\$NoBrowser\b/);
  assert.ok(t.includes('Start Menu\\Programs\\CSR'), 'Start menu folder');
  assert.match(t, /\[InternetShortcut\]/);
  assert.match(t, /Start-Process "http:\/\/127\.0\.0\.1:/);
  assert.match(t, /uninstall\.cmd/);
});

test('uninstaller removes the Start menu folder', () => {
  const t = read('scripts', 'uninstall.ps1');
  assert.ok(t.includes('Start Menu\\Programs\\CSR'), 'Start menu folder');
});

test('install.cmd / uninstall.cmd: CRLF, self-elevating, bypass, pause', () => {
  for (const f of ['install.cmd', 'uninstall.cmd']) {
    const raw = fs.readFileSync(path.join(ROOT, 'scripts', f));
    const t = raw.toString('utf8');
    assert.ok(t.includes('\r\n') && !/[^\r]\n/.test(t), `${f}: CRLF only`);
    assert.match(t, /fltmc/);
    assert.match(t, /-Verb RunAs/);
    assert.match(t, /-ExecutionPolicy Bypass/);
    assert.match(t, /pause/);
  }
  const u = read('scripts', 'uninstall.cmd');
  assert.match(u, /-RemoveData/);
  assert.match(u, /cd \/d "%TEMP%"/, 'leaves the program folder before it is deleted');
  assert.match(u, /& pause & exit \/b/, 'last command on one line, the file may be gone afterwards');
});

test('package contains the launchers and the new README text', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'csrwin-'));
  const exe = path.join(tmp, 'csr.exe');
  fs.writeFileSync(exe, 'MZ fake');
  const ver = '0.0.0-uxtest';
  const r = spawnSync('bash', ['scripts/package-windows.sh', ver, exe], { encoding: 'utf8', cwd: ROOT });
  assert.equal(r.status, 0, r.stderr);
  const zip = r.stdout.trim().split('\n').pop();
  const py = spawnSync('python3', ['-I', '-c',
    'import sys,zipfile; z=zipfile.ZipFile(sys.argv[1]); n=z.namelist(); print("\\n".join(n)); ' +
    'print("---"); print([x for x in n if x.endswith("README-windows.txt")] and z.read([x for x in n if x.endswith("README-windows.txt")][0]).decode())',
    zip], { encoding: 'utf8' });
  fs.rmSync(zip, { force: true });
  fs.rmSync(tmp, { recursive: true, force: true });
  assert.equal(py.status, 0, py.stderr);
  for (const re of [/install\.cmd$/m, /uninstall\.cmd$/m, /install\.ps1$/m, /uninstall\.ps1$/m]) assert.match(py.stdout, re);
  assert.match(py.stdout, /double-click install\.cmd/);
  assert.match(py.stdout, /light client/);
});

test('csr.exe hint when double-clicked from the zip', () => {
  const t = read('runner', 'src', 'main.rs');
  assert.match(t, /fn install_hint\(/);
  assert.match(t, /fn double_click_hint\(/);
  assert.match(t, /double-click install\.cmd/);
});
