// csr-services-detail, phase 2: wiring of the dashboard (helpers are tested in p2helpers.test.mjs).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const asset = (f) => fs.readFileSync(path.join(ROOT, 'runner', 'assets', f), 'utf8');

test('index.html loads svc-detail.js before app.js', () => {
  const html = asset('index.html');
  const i1 = html.indexOf('<script src="/svc-detail.js"></script>');
  const i2 = html.indexOf('<script src="/app.js"></script>');
  assert.ok(i1 > 0 && i2 > i1);
});

test('the Services page uses the helpers, the verify route and the new texts', () => {
  const a = asset('app.js');
  for (const fn of ['displayName', 'facts', 'approveWarning', 'verifyText', 'CLAIM_NOTE', 'explorerTxUrl']) {
    assert.match(a, new RegExp(`CSRSvc\\.${fn}\\b`), `app.js uses CSRSvc.${fn}`);
  }
  assert.match(a, /\/verify/);
  assert.match(a, /Verify binary/);
  assert.match(a, /Confirm approve/);
  assert.match(a, /'Details'|"Details"/);
  assert.match(a, /links\(\)\.explorer/);
  assert.match(a, /colSpan\s*=|colspan/i, 'the detail panel is a table row spanning all columns');
});

test('no new innerHTML: details are built from text nodes', () => {
  assert.ok((asset('app.js').match(/innerHTML/g) || []).length <= 1, 'only the existing world map use');
  assert.doesNotMatch(asset('svc-detail.js'), /innerHTML/);
});

test('styles exist', () => {
  const css = asset('app.css');
  assert.match(css, /\.svc-detail/);
});

test('the catalog cards of services that are not added yet show the same facts', () => {
  const a = asset('app.js');
  assert.ok((a.match(/CSRSvc\.facts\(/g) || []).length >= 2, 'facts() used for added services and for catalog items');
});
