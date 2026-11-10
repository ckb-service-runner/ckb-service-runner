import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const asset = (f) => fs.readFileSync(path.join(ROOT, 'runner', 'assets', f), 'utf8');

test('Services page has the catalog sections', () => {
  const a = asset('app.js');
  assert.match(a, /Available on chain/);
  assert.match(a, /Seen on chain/);
  assert.match(a, /Add & approve/);
  assert.match(a, /\/api\/catalog/);
});

test('links are https only and never fetched', () => {
  const a = asset('app.js');
  assert.match(a, /noopener noreferrer nofollow/);
  assert.match(a, /startsWith\(['"]https:\/\/['"]\)/);
  assert.doesNotMatch(a, /fetch\(\s*(item|entry|m|s)\.(url|source)/);
});

test('descriptions are rendered as text', () => {
  const a = asset('app.js');
  assert.doesNotMatch(a, /innerHTML\s*=\s*[^;]*description/);
});

test('Overview shows the new-service notice', () => {
  const a = asset('app.js');
  assert.match(a, /New service available/);
  assert.match(a, /\/api\/catalog\/seen/);
});

test('assets stay within 200 KB', () => {
  const dir = path.join(ROOT, 'runner', 'assets');
  const total = fs.readdirSync(dir).filter((f) => /\.(js|css|html)$/.test(f))
    .reduce((n, f) => n + fs.statSync(path.join(dir, f)).size, 0);
  assert.ok(total <= 200 * 1024, `${total} bytes`);
});
