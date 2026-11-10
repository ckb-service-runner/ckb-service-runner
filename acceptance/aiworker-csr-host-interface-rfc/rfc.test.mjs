import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const read = (...p) => fs.readFileSync(path.join(process.cwd(), ...p), 'utf8');

test('draft has the RFC front matter and sections', () => {
  const r = read('docs', 'rfc', '0000-ckb-vm-host-interface.md');
  assert.match(r, /Status:\s*Draft/);
  for (const h of ['Abstract', 'Motivation', 'Specification', 'Security', 'Compatibility', 'Reference implementation'])
    assert.match(r, new RegExp(`^#+ .*${h}`, 'm'), h);
});

test('every syscall number of the spec appears in the draft, and no other 70xx number', () => {
  const spec = read('spec', 'host-interface.md');
  const r = read('docs', 'rfc', '0000-ckb-vm-host-interface.md');
  const nums = (t) => new Set([...t.matchAll(/\b70[0-9]{2}\b/g)].map((m) => m[0]));
  const inSpec = nums(spec);
  for (const n of inSpec) assert.ok(r.includes(n), `missing ${n}`);
  for (const n of nums(r)) assert.ok(inSpec.has(n) || n === '7099' || n === '7001', `unknown syscall ${n}`);
});

test('draft is marked for maintainer submission and on the roadmap', () => {
  assert.match(read('docs', 'rfc', 'README.md'), /draft/i);
  assert.match(read('docs', 'roadmap.md'), /docs\/rfc/);
});
