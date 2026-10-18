import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ROOT } from '../a1/lib.mjs';

test('network page is linked and documented', () => {
  const html = fs.readFileSync(path.join(ROOT, 'runner', 'assets', 'index.html'), 'utf8');
  assert.ok(html.includes('href="#/network"'), 'navigation link #/network missing');
  assert.ok(fs.existsSync(path.join(ROOT, 'docs', 'milestones', 'A1plus.md')));
  assert.match(fs.readFileSync(path.join(ROOT, 'CHANGELOG.md'), 'utf8'), /A1\+|A1plus/);
  const docs = ['spec/ping-protocol.md', 'spec/monitor-public-api.md']
    .filter((f) => fs.existsSync(path.join(ROOT, f)))
    .map((f) => fs.readFileSync(path.join(ROOT, f), 'utf8'))
    .join('\n');
  assert.ok(docs.includes('/v1/network') && docs.includes('/v1/nodes/'), 'monitor public API not documented');
});
