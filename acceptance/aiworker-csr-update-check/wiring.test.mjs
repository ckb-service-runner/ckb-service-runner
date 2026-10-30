// csr-update-check: the update module is reachable from the CLI, the dashboard, and the install units.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const exists = (...p) => fs.existsSync(path.join(ROOT, ...p));

test('update.rs includes the acceptance test module', () => {
  assert.match(src('update.rs'), /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-update-check\/update_tests\.rs"\]/);
  assert.match(src('main.rs'), /^mod update;/m);
});

test('CLI and dashboard routes', () => {
  assert.match(src('main.rs'), /Update\b/);
  const r = src('web/routes.rs');
  assert.match(r, /"\/api\/update"/);
  assert.match(r, /"\/api\/update\/install"/);
  assert.match(src('web/auth.rs'), /\/api\/update\//, 'update writes need the dashboard password');
  const js = read('runner', 'assets', 'app.js');
  assert.match(js, /Check for update/);
  assert.match(js, /\/api\/update\/install/);
});

test('the root installer and its systemd units exist', () => {
  assert.ok(exists('config', 'csr-update.path'));
  assert.ok(exists('config', 'csr-update.service'));
  assert.ok(exists('scripts', 'install-update.sh'));
  const sh = read('scripts', 'install-update.sh');
  assert.match(sh, /csr update verify/, 'the root script lets the installed binary verify the staged one');
  assert.match(read('config', 'csr-update.service'), /ExecStart=.*install-update\.sh/);
  assert.match(read('config', 'csr-update.path'), /PathExists=|PathChanged=/);
});

test('the release process is documented', () => {
  assert.ok(exists('scripts', 'release.sh'));
  const t = read('docs', 'install.md') + (exists('docs', 'release.md') ? read('docs', 'release.md') : '');
  assert.match(t, /SHA256SUMS\.sig/);
  assert.match(t, /csr-update\.path/);
});
