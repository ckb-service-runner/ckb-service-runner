import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('publish command exists', () => {
  const m = read('runner', 'src', 'main.rs');
  assert.match(m, /Publish\s*\{/);
  assert.match(m, /code_outpoint|code-outpoint/);
});

test('developer guide and README state the no-update guarantee', () => {
  assert.match(read('docs', 'publishing-a-service.md'), /service ID/);
  assert.match(read('README.md'), /no runner update/i);
  assert.match(read('CHANGELOG.md'), /catalog/i);
});

test('devnet testing steps cover catalog, composite service and update', () => {
  const t = read('docs', 'testing.md');
  assert.match(t, /catalog/i);
  assert.match(t, /hello-js/);
  assert.match(t, /Update available/);
});
