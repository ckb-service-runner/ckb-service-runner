// v0.4.0: redesigned Overview and Node pages, Auto renew setting, light/dark theme.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, setup, startCsr, getJson, cleanup } from '../a1/lib.mjs';

after(cleanup);
const A = (f) => fs.readFileSync(path.join(ROOT, 'runner', 'assets', f), 'utf8');

test('assets: theme toggle, both themes, auto renew switch, node hero', () => {
  const html = A('index.html'), js = A('app.js'), css = A('app.css');
  assert.match(html, /id="theme-toggle"/);
  assert.match(css, /:root\[data-theme=dark\]/);
  assert.match(css, /\[hidden\]\{display:none !important\}/);
  for (const s of ["registry: { auto_renew:", "role', 'switch'", 'next_renewal_ms', 'nodeHero', 'tip_timestamp_ms', 'epoch_index']) {
    assert.ok(js.includes(s), `app.js must contain ${s}`);
  }
});

test('API: status links and hostname, node epoch and tip time, registry auto_renew default', async () => {
  const env = await setup('v04-api');
  await startCsr([...env.args, 'run'], { healthUrl: `${env.base}/healthz` });
  const s = (await getJson(`${env.base}/api/status`)).json;
  assert.ok('hostname' in s);
  assert.equal(s.links.explorer, 'https://testnet.explorer.nervos.org');
  assert.equal(s.links.nodes_list, null);
  const n = (await getJson(`${env.base}/api/node`)).json;
  for (const k of ['epoch_index', 'epoch_length', 'tip_timestamp_ms', 'node_id', 'rpc_url']) assert.ok(k in n, `node.${k}`);
  const cfg = (await getJson(`${env.base}/api/settings`)).json;
  assert.equal(cfg.registry.auto_renew, true, 'auto renew is on by default');
});
