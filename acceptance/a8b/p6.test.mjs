// A8b Phase 2: dashboard polish – the setup text, the runner address, copying, and the project logo.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, startCsr, cleanup } from '../a1/lib.mjs';

after(cleanup);

const read = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');
const ASSETS = path.join(ROOT, 'runner', 'assets');
const LOGOS = ['logo.png', 'logo-full.png', 'favicon.png'];
const expected = (name) => Buffer.from(read(`acceptance/a8b/${name}.b64`).replace(/\s+/g, ''), 'base64');

test('wallet page: the real setup cost, the full address, copying that works over http', () => {
  const js = read('runner/assets/app.js');
  assert.doesNotMatch(js, /about 500 CKB/i, 'the setup text must not say about 500 CKB');
  assert.doesNotMatch(js, /161 CKB/, 'the registry cell does not cost 161 CKB');
  assert.match(js, /700 CKB/);
  assert.match(js, /540 CKB/);
  assert.match(js, /addr-full/, 'the runner address uses the class addr-full');
  assert.doesNotMatch(js, /copyBtn\([^)]*addr/, 'the runner address has no copy button any more');
  assert.match(js, /execCommand\(\s*['"]copy['"]\s*\)/, 'copying falls back to execCommand("copy") without navigator.clipboard');
  const css = read('runner/assets/app.css');
  const rule = css.match(/\.addr-full\s*\{([^}]*)\}/);
  assert.ok(rule, 'app.css needs a .addr-full rule');
  assert.match(rule[1], /overflow-wrap\s*:\s*anywhere|word-break\s*:\s*break-all/);
});

test('the project logo is in the assets, the header, the login page and the favicon', () => {
  for (const name of LOGOS) {
    const f = path.join(ASSETS, name);
    assert.ok(fs.existsSync(f), `runner/assets/${name} missing`);
    assert.ok(fs.readFileSync(f).equals(expected(name)), `runner/assets/${name} differs from acceptance/a8b/${name}.b64`);
  }
  const html = read('runner/assets/index.html');
  assert.match(html, /<link[^>]+rel="icon"[^>]+href="\/favicon\.png"|<link[^>]+href="\/favicon\.png"[^>]+rel="icon"/);
  assert.match(html, /<header[^]*<img[^>]+src="\/logo\.png"[^>]*>[^]*<\/header>/, 'header shows /logo.png');
  assert.match(html, /<img[^>]+src="\/logo\.png"[^>]+alt="[^"]*"|<img[^>]+alt="[^"]*"[^>]+src="\/logo\.png"/, 'logo needs an alt attribute');
  assert.match(read('runner/assets/app.js'), /\/logo-full\.png/, 'the login page shows the full logo');
  assert.match(read('runner/assets/app.css'), /\.logo\b/);
});

test('logo files are served as image/png', async () => {
  const env = await setup('a8b-logo');
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  for (const name of LOGOS) {
    const r = await fetch(`${env.base}/${name}`);
    assert.equal(r.status, 200, name);
    assert.match(r.headers.get('content-type') || '', /^image\/png/, name);
    assert.ok(Buffer.from(await r.arrayBuffer()).equals(expected(name)), name);
  }
});
