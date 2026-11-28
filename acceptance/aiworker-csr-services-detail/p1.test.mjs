// csr-services-detail, phase 1: wiring of the verify logic, the route and the extra service fields.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('verify.rs exists, is a module of the binary and includes the acceptance tests', () => {
  assert.match(read('runner', 'src', 'main.rs'), /^mod verify;$/m);
  const v = read('runner', 'src', 'verify.rs');
  assert.match(v, /#\[path = "\.\.\/\.\.\/acceptance\/aiworker-csr-services-detail\/verify_tests\.rs"\]\s*mod aiworker_sd_verify_tests;/);
  for (const name of ['pub enum CellRead', 'pub enum Status', 'pub struct Report', 'pub fn verify(', 'pub async fn verify_service(']) {
    assert.ok(v.includes(name), `verify.rs has ${name}`);
  }
});

test('GET /api/services/{id}/verify is a read-only route next to the service actions', () => {
  const r = read('runner', 'src', 'web', 'routes.rs');
  assert.match(r, /\.route\("\/api\/services\/\{\w+\}\/verify",\s*get\(/);
  assert.match(r, /\.route\("\/api\/services\/\{hash\}\/\{verb\}",\s*post\(service_action\)\)/, 'the action route is unchanged');
  assert.match(r, /verify::verify_service/);
});

test('the service list carries the code cell and the manifest cell', () => {
  const reg = read('runner', 'src', 'registry.rs');
  const entry = reg.slice(reg.indexOf('fn entry('), reg.indexOf('pub fn list('));
  assert.match(entry, /"outpoint"/);
  assert.match(entry, /"manifest_outpoint"/);
  assert.match(reg, /pub\(crate\) async fn live_data|pub async fn live_data/, 'live_data is reachable from verify.rs');
});

test('verify never talks to anything but the own node', () => {
  const v = read('runner', 'src', 'verify.rs');
  assert.doesNotMatch(v, /reqwest|hyper::Client|https?:\/\/[a-z]/i);
});
