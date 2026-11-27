// csr-07-security, phase 3: installer and uninstaller (S2-S5). Content checks, plus a syntax parse when pwsh exists.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const inst = read('scripts', 'install.ps1');
const un = read('scripts', 'uninstall.ps1');

// The text of a PowerShell function, from "function Name" to its closing brace.
function fnBody(src, name) {
  const re = new RegExp(`function ${name}\\s*(?:\\([^)]*\\))?\\s*\\{`);
  const m = re.exec(src);
  if (!m) return null;
  let i = m.index + m[0].length;
  let depth = 1;
  while (i < src.length && depth > 0) {
    const c = src[i++];
    if (c === '{') depth++;
    else if (c === '}') depth--;
  }
  return src.slice(m.index, i);
}

const pwsh = spawnSync('pwsh', ['-NoProfile', '-Command', '$PSVersionTable.PSVersion.Major'], { encoding: 'utf8' });
const HAVE_PWSH = pwsh.status === 0;
function parses(file) {
  const cmd = `$e=$null; $t=$null; [void][System.Management.Automation.Language.Parser]::ParseFile('${file}', [ref]$t, [ref]$e); if ($e.Count) { $e | ForEach-Object { $_.ToString() }; exit 1 }`;
  return spawnSync('pwsh', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' });
}

test('PowerShell syntax is valid (when pwsh is available)', { skip: !HAVE_PWSH && 'pwsh not installed' }, () => {
  for (const f of ['install.ps1', 'uninstall.ps1']) {
    const r = parses(path.join(ROOT, 'scripts', f));
    assert.equal(r.status, 0, `${f}: ${r.stdout}${r.stderr}`);
  }
});

test('the installer creates one service per network under a virtual account', () => {
  assert.ok(fnBody(inst, 'Register-CsrService'), 'function Register-CsrService');
  assert.match(inst, /sc\.exe/i);
  assert.match(inst, /\bcreate\b/);
  assert.match(inst, /obj=/);
  assert.match(inst, /NT SERVICE\\CSR-/);
  assert.match(inst, /windows-service/);
  assert.match(inst, /failureflag/);
  assert.match(inst, /delayed-auto/);
  assert.match(inst, /SeChangeNotifyPrivilege/);
});

test('no SYSTEM task is created any more, the legacy task is migrated', () => {
  assert.doesNotMatch(inst, /New-ScheduledTaskPrincipal/);
  assert.doesNotMatch(inst, /Register-ScheduledTask/);
  assert.doesNotMatch(inst, /RunLevel/);
  assert.ok(fnBody(inst, 'Remove-LegacyTask'), 'function Remove-LegacyTask');
  assert.match(inst, /Unregister-ScheduledTask/);
  assert.match(inst, /Stop-Service|sc\.exe['"]?\s*,?\s*@?\(?\s*['"]?stop/i);
});

test('SYSTEM is not granted on the data root', () => {
  const body = fnBody(inst, 'Restrict-DataRoot');
  assert.ok(body, 'function Restrict-DataRoot');
  for (const line of inst.split(/\r?\n/)) {
    if (/S-1-5-18/.test(line)) assert.match(line, /remove/i, `SYSTEM may only be removed: ${line.trim()}`);
  }
  assert.match(body, /S-1-5-32-544/);
  assert.match(body, /NT SERVICE\\CSR-/);
  assert.match(body, /\/inheritance:r/);
});

test('the node key file gets one read-only entry', () => {
  const body = fnBody(inst, 'Grant-NodeKeyRead');
  assert.ok(body, 'function Grant-NodeKeyRead');
  assert.match(body, /icacls/i);
  assert.match(body, /\(R\)/);
  assert.match(body, /NT SERVICE\\CSR-/);
  assert.doesNotMatch(body, /\(OI\)|\(CI\)|:F\b|\(F\)|\(M\)/, 'read only, no inheritance, no write');
});

test('services start only after the permissions are set (order)', () => {
  const main = inst.slice(inst.indexOf('function Install-Csr'));
  const idx = (re) => { const m = re.exec(main); return m ? m.index : -1; };
  const reg = idx(/Register-CsrService\s+\$/);
  const acl = idx(/Restrict-DataRoot/);
  const key = idx(/Grant-NodeKeyRead\s+\$/);
  const start = idx(/Start-CsrService\s+\$/);
  const ident = idx(/Set-Identity\s+\$/);
  for (const [n, v] of Object.entries({ reg, acl, key, start, ident })) assert.ok(v >= 0, `${n} call missing in Install-Csr`);
  assert.ok(ident < reg, 'identity step before the services are created');
  assert.ok(reg < acl, 'services exist before the ACLs are set');
  assert.ok(acl < key, 'data root before key');
  assert.ok(key < start, 'key permission before start');
  assert.ok(fnBody(inst, 'Start-CsrService'), 'function Start-CsrService');
});

test('key search uses known places only, wider search is opt-in and asks', () => {
  const body = fnBody(inst, 'Find-NodeKeys');
  assert.ok(body, 'function Find-NodeKeys');
  assert.doesNotMatch(body, /USERPROFILE|LOCALAPPDATA/);
  assert.match(body, /RunningNodeDirs/);
  assert.match(body, /Neuron/);
  assert.match(inst, /\[switch\]\$SearchWide/);
  const wide = fnBody(inst, 'Find-NodeKeysWide');
  assert.ok(wide, 'function Find-NodeKeysWide');
  assert.match(wide, /USERPROFILE/);
  assert.match(wide, /LOCALAPPDATA/);
  assert.match(inst, /Read-Host|PromptForChoice/);
  const ident = fnBody(inst, 'Set-Identity');
  assert.ok(ident, 'function Set-Identity');
  assert.match(ident, /key candidate: /);
});

test('the firewall rule exists only for inbound ping modes', () => {
  const body = fnBody(inst, 'Set-PingFirewallRule');
  assert.ok(body, 'function Set-PingFirewallRule');
  assert.match(body, /ping_mode/);
  assert.match(body, /inbound/);
  assert.match(body, /Remove-NetFirewallRule/);
  assert.match(body, /outbound/);
});

test('the uninstaller removes the key entry before it deletes the service', () => {
  assert.match(un, /NT SERVICE\\CSR-/);
  assert.match(un, /sc\.exe/i);
  assert.match(un, /\bdelete\b/);
  assert.match(un, /icacls/i);
  assert.match(un, /\/remove/);
  assert.match(un, /Unregister-ScheduledTask/);
  assert.match(un, /Remove-NetFirewallRule/);
  assert.ok(un.search(/\/remove/) < un.search(/\bdelete\b/), 'ACE removal comes before the service delete');
  assert.match(un, /RemoveData/);
});

test('docs describe the service, not the SYSTEM task', () => {
  const d = read('docs', 'windows.md');
  assert.match(d, /NT SERVICE\\CSR-/);
  assert.match(d, /Get-Service/);
  assert.match(d, /services\.msc/);
  assert.match(d, /SearchWide/);
  assert.doesNotMatch(d, /as SYSTEM/);
  assert.doesNotMatch(d, /Get-ScheduledTask CSR-/);
});

test('a running csr.exe is stopped robustly (issue 32: "Access is denied" from Process.Kill)', () => {
  for (const [name, src] of [['install.ps1', inst], ['uninstall.ps1', un]]) {
    assert.doesNotMatch(src, /\.Kill\(\)/, `${name} must not call Process.Kill()`);
    assert.match(src, /Stop-RunningCsr/, name);
    assert.match(src, /taskkill/i, name);
  }
});
