// csr-windows, phase 2: PowerShell installer and uninstaller (W1–W5). Content checks, plus a syntax parse
// when pwsh is installed. Nothing is executed on Windows here.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const INSTALL = path.join(ROOT, 'scripts', 'install.ps1');
const UNINSTALL = path.join(ROOT, 'scripts', 'uninstall.ps1');

const pwsh = spawnSync('pwsh', ['-NoProfile', '-Command', '$PSVersionTable.PSVersion.Major'], { encoding: 'utf8' });
const HAVE_PWSH = pwsh.status === 0;

function parse(file) {
  const cmd = `$e=$null; $t=$null; [void][System.Management.Automation.Language.Parser]::ParseFile('${file}', [ref]$t, [ref]$e); if ($e.Count) { $e | ForEach-Object { $_.ToString() }; exit 1 }`;
  return spawnSync('pwsh', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' });
}

test('installer and uninstaller exist', () => {
  assert.ok(fs.existsSync(INSTALL));
  assert.ok(fs.existsSync(UNINSTALL));
});

test('PowerShell syntax is valid (when pwsh is available)', { skip: !HAVE_PWSH && 'pwsh not installed' }, () => {
  for (const f of [INSTALL, UNINSTALL]) {
    const r = parse(f);
    assert.equal(r.status, 0, `${f}: ${r.stdout}${r.stderr}`);
  }
});

test('installer: strict mode, admin check, beta header', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /Set-StrictMode -Version Latest/);
  assert.match(t, /\$ErrorActionPreference\s*=\s*'Stop'/);
  assert.match(t, /Administrator/);
  assert.match(t, /beta/i);
});

test('installer: required functions and parameters', () => {
  const t = read('scripts', 'install.ps1');
  for (const f of ['Get-ChainNetwork', 'Get-InstancePorts', 'Invoke-NodeRpc', 'Get-RpcUrlFromCkbToml',
    'Find-NodeKeys', 'Find-Nodes', 'Write-InstanceConfig', 'Set-PeerUrl', 'Register-CsrTask', 'Show-Summary']) {
    assert.match(t, new RegExp(`function\\s+${f}\\b`, 'i'), f);
  }
  for (const p of ['Rpc', 'CkbDir', 'NoRanking', 'Source']) assert.match(t, new RegExp(`\\$${p}\\b`), p);
});

test('installer: same networks and ports as Linux', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /ckb_testnet/);
  for (const p of ['8130', '8131', '8140', '8141', '8114']) assert.ok(t.includes(p), p);
  assert.match(t, /csr-\$\{?net/i);
  assert.match(t, /confirm-mainnet/);
  assert.match(t, /set-password/);
  assert.match(t, /--password-stdin/);
  assert.match(t, /peer_url/);
  assert.match(t, /ping_mode/);
  assert.match(t, /p2p_key_file/);
});

test('installer: data root, binary location, Neuron hints', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /ProgramData/);
  assert.match(t, /ProgramFiles/);
  assert.match(t, /Neuron/);
  assert.match(t, /light client/i);
  assert.match(t, /"Net"|'Net'|\bNet\b module/);
});

test('installer: scheduled task per network as SYSTEM, at startup, restarts', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /Register-ScheduledTask/);
  assert.match(t, /CSR-/);
  assert.match(t, /-AtStartup/);
  assert.match(t, /SYSTEM/);
  assert.match(t, /RestartCount/);
  assert.match(t, /ExecutionTimeLimit/);
});

test('installer: firewall opens only the ping port', () => {
  const t = read('scripts', 'install.ps1');
  assert.match(t, /New-NetFirewallRule/);
  assert.match(t, /CSR ping/);
  const rules = t.split('\n').filter((l) => /New-NetFirewallRule/.test(l)).join('\n');
  assert.doesNotMatch(rules, /8130|8140/, 'never open a dashboard port');
});

test('installer does not edit ckb.toml', () => {
  const t = read('scripts', 'install.ps1');
  assert.doesNotMatch(t, /(Set-Content|Out-File|Add-Content)[^\n]*ckb\.toml/i);
});

test('uninstaller: removes tasks and rules, keeps data unless confirmed', () => {
  const t = read('scripts', 'uninstall.ps1');
  assert.match(t, /Set-StrictMode -Version Latest/);
  assert.match(t, /Unregister-ScheduledTask/);
  assert.match(t, /Remove-NetFirewallRule/);
  assert.match(t, /\$RemoveData\b/);
  assert.match(t, /DELETE/);
  assert.match(t, /Administrator/);
});
