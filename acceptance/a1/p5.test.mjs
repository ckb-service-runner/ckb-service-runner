import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, startCsr, cleanup } from './lib.mjs';

after(cleanup);

const ASSETS = path.join(ROOT, 'runner', 'assets');
function walk(d) {
  return fs.readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]));
}
const ALLOWED_URLS = new Set(['http://www.w3.org/2000/svg', 'http://www.w3.org/1999/xlink', 'http://www.w3.org/1999/xhtml']);

test('asset budget and no external URLs', () => {
  const files = walk(ASSETS);
  assert.ok(files.length > 0, 'runner/assets is empty');
  const total = files.reduce((s, f) => s + fs.statSync(f).size, 0);
  assert.ok(total <= 200 * 1024, `assets are ${total} bytes (limit 204800)`);
  for (const f of files) {
    const t = fs.readFileSync(f, 'latin1');
    for (const m of t.matchAll(/https?:\/\/[^\s"'`)<>]+/g)) {
      assert.ok(ALLOWED_URLS.has(m[0]), `${path.relative(ROOT, f)} contains external URL ${m[0]}`);
    }
  }
});

test('index.html structure', () => {
  const html = fs.readFileSync(path.join(ASSETS, 'index.html'), 'utf8');
  for (const r of ['#/overview', '#/node', '#/resources', '#/settings', '#/logs']) {
    assert.ok(html.includes(`href="${r}"`), `navigation link ${r} missing`);
  }
  assert.match(html, /id="network-badge"/);
  assert.match(html, /<meta[^>]+name="viewport"/i);
  assert.doesNotMatch(html, /<script(?![^>]*\bsrc=)[^>]*>/i, 'inline <script> blocks are not allowed');
  assert.doesNotMatch(html, /\son[a-z]+\s*=/i, 'inline event handlers are not allowed');
  assert.doesNotMatch(html, /\sstyle\s*=/i, 'style= attributes are not allowed');
});

test('dashboard is served from the binary with security headers', async () => {
  const env = await setup('p5');
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  const r = await fetch(`${env.base}/`);
  assert.equal(r.status, 200);
  assert.match(r.headers.get('content-type') || '', /text\/html/);
  const csp = r.headers.get('content-security-policy') || '';
  assert.match(csp, /default-src 'self'/);
  assert.match(csp, /script-src 'self'/);
  assert.match(csp, /frame-ancestors 'none'/);
  assert.equal(r.headers.get('x-content-type-options'), 'nosniff');
  const body = await r.text();
  assert.equal(body, fs.readFileSync(path.join(ASSETS, 'index.html'), 'utf8'), 'served index.html differs from runner/assets/index.html');
  const refs = [...body.matchAll(/(?:src|href)="([^"#][^"]*)"/g)].map((m) => m[1]).filter((x) => !x.startsWith('data:'));
  assert.ok(refs.some((x) => x.endsWith('.js')), 'no script referenced');
  assert.ok(refs.some((x) => x.endsWith('.css')), 'no stylesheet referenced');
  for (const ref of refs) {
    const u = new URL(ref, `${env.base}/`);
    assert.equal(u.origin, new URL(env.base).origin, `${ref} is not same-origin`);
    const a = await fetch(u);
    assert.equal(a.status, 200, `${ref} -> ${a.status}`);
    const ct = a.headers.get('content-type') || '';
    if (ref.endsWith('.js')) assert.match(ct, /javascript/);
    if (ref.endsWith('.css')) assert.match(ct, /text\/css/);
  }
});
