// A6 Phase 2: "CSR monitor" naming, world map on the CSR monitor page, docs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT } from '../a1/lib.mjs';

const A = (f) => fs.readFileSync(path.join(ROOT, 'runner', 'assets', f), 'utf8');
const R = (f) => fs.readFileSync(path.join(ROOT, f), 'utf8');

test('index.html: map script before app.js, renamed tab', () => {
  const html = A('index.html');
  const i1 = html.indexOf('<script src="/worldmap.js"></script>');
  const i2 = html.indexOf('<script src="/app.js"></script>');
  assert.ok(i1 > 0 && i2 > i1, 'worldmap.js must be loaded right before app.js');
  assert.match(html, /<a href="#\/network">CSR monitor<\/a>/);
  assert.doesNotMatch(html, />Network<\/a>/);
});

test('app.js: names, map card, wording', () => {
  const js = A('app.js');
  assert.match(js, /card\('CSR monitor'\)/, 'Overview card must be titled CSR monitor');
  assert.doesNotMatch(js, /card\('Ranking'\)/);
  assert.match(js, /card\('Map'\)/);
  assert.match(js, /CSRWorldMap/);
  assert.match(js, /\.svg\(/);
  assert.match(js, /map-info/);
  assert.match(js, /href = '#\/network'|href='#\/network'|'#\/network'/);
  assert.match(js, /\['ranking\.join', 'Join the CSR monitor', 'bool'\]/);
  assert.ok(js.includes('Join the CSR monitor to see stats'));
  assert.ok(!/Join the ranking/i.test(js), 'old wording left in app.js');
  for (const s of ['Natural Earth', 'DB-IP', 'CC BY 4.0']) assert.ok(js.includes(s), `credit text ${s} missing`);
});

test('app.css: map styles for both themes', () => {
  const css = A('app.css');
  for (const sel of ['.wm-c', '.wm-dot', '.wm-shadow']) assert.ok(css.includes(sel), sel);
  for (const v of ['--wm-land:', '--wm-border:', '--wm-shadow:', '--wm-dot:']) {
    assert.ok(css.split(v).length - 1 >= 2, `${v} must be set for light and dark theme`);
  }
  assert.match(css, /:root\[data-theme=dark\]\{[^}]*--wm-land:/);
});

test('docs', () => {
  assert.ok(fs.existsSync(path.join(ROOT, 'docs', 'milestones', 'A6.md')));
  assert.match(R('docs/milestones/A6.md'), /CSR monitor/);
  assert.match(R('CHANGELOG.md'), /A6/);
  const api = R('spec/monitor-public-api.md');
  assert.ok(api.includes('/v1/locations') && api.includes('locations_available'));
});
