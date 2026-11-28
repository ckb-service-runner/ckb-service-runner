// csr-071-crowdcell, phase 1: the crowdcell-finalize service, its logic crate, the csr-svc wrappers, the manifest.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const exists = (...p) => fs.existsSync(path.join(ROOT, ...p));
const count = (s, needle) => s.split(needle).length - 1;

test('both crates are workspace members with the MIT licence', () => {
  const ws = read('services', 'rust', 'Cargo.toml');
  assert.match(ws, /"crowdcell-finalize-logic"/);
  assert.match(ws, /"crowdcell-finalize"/);
  for (const c of ['crowdcell-finalize-logic', 'crowdcell-finalize']) {
    const t = read('services', 'rust', c, 'Cargo.toml');
    assert.match(t, /^license\s*=\s*"MIT"/m, `${c} has license = "MIT"`);
    assert.match(t, new RegExp(`^name\\s*=\\s*"${c}"`, 'm'));
  }
  const lock = read('services', 'rust', 'Cargo.lock');
  assert.match(lock, /name = "crowdcell-finalize-logic"/);
  assert.match(lock, /name = "crowdcell-finalize"/);
});

test('the logic crate is no_std outside tests, has the agreed API and includes the acceptance tests', () => {
  const lib = read('services', 'rust', 'crowdcell-finalize-logic', 'src', 'lib.rs');
  assert.match(lib, /#!\[cfg_attr\(not\(test\), no_std\)\]/);
  for (const item of ['pub fn parse_payload', 'pub fn parse_campaign', 'pub fn decide', 'pub fn finalized_data',
    'pub fn build_skeleton', 'pub enum Outcome', 'pub enum PayloadError', 'pub enum BuildError', 'pub struct Payload',
    'pub struct Campaign', 'pub const MAX_SCRIPT_LEN']) {
    assert.ok(lib.includes(item), `lib.rs has ${item}`);
  }
  assert.ok(lib.includes('#[path = "../../../../acceptance/aiworker-csr-071-crowdcell/logic_tests.rs"]'));
  assert.ok(lib.includes('mod aiworker_071_logic_tests;'));
  // the status offsets are the ones of CrowdCell's campaign cell data (contracts/campaign): status at byte 56
  assert.match(lib, /STATUS_OFFSET\s*:\s*usize\s*=\s*56/);
  assert.match(lib, /CAMPAIGN_MIN_SIZE\s*:\s*usize\s*=\s*65/);
});

test('csr-svc exposes the four host calls the service needs, with the numbers of spec/host-interface.md', () => {
  const s = read('services', 'rust', 'csr-svc', 'src', 'lib.rs');
  for (const [name, n] of [['BLOCK_INFO', 7002], ['CHAIN_READ', 7003], ['CELLS_QUERY', 7004], ['TX_SKELETON_PUT', 7030]]) {
    assert.match(s, new RegExp(`pub const ${name}: u64 = ${n};`));
  }
  for (const f of ['pub fn block_info', 'pub fn chain_read', 'pub fn cells_query', 'pub fn tx_skeleton_put']) {
    assert.ok(s.includes(f), `csr-svc has ${f}`);
  }
});

test('the service binary is thin, stateless and writes a skeleton only when it finalizes', () => {
  const m = read('services', 'rust', 'crowdcell-finalize', 'src', 'main.rs');
  assert.match(m, /#!\[no_std\]/);
  assert.match(m, /#!\[no_main\]/);
  for (const call of ['csr_svc::job_payload', 'csr_svc::cells_query', 'csr_svc::chain_read', 'csr_svc::block_info',
    'csr_svc::result_put', 'parse_payload', 'parse_campaign', 'decide(', 'finalized_data', 'build_skeleton']) {
    assert.ok(m.includes(call), `main.rs uses ${call}`);
  }
  assert.equal(count(m, 'tx_skeleton_put('), 1, 'tx_skeleton_put is called in one place only');
  for (const banned of ['kv_put', 'kv_get', 'csr_svc::sign', 'random_bytes', 'unsafe']) {
    assert.ok(!m.includes(banned), `main.rs does not use ${banned}`);
  }
  // cells_query by type script (script_type 1); chain_read kind 0 (output) and 1 (data)
  assert.match(m, /cells_query\(&payload\.type_script, 1,/);
  assert.match(m, /chain_read\(0,/);
  assert.match(m, /chain_read\(1,/);
});

test('manifest of the service', () => {
  const t = read('services', 'manifests', 'crowdcell-finalize.toml');
  assert.match(t, /^name\s*=\s*"crowdcell-finalize"/m);
  assert.match(t, /^language\s*=\s*"rust"/m);
  assert.match(t, /^elf\s*=\s*"\.\.\/rust\/target\/riscv64imac-unknown-none-elf\/release\/crowdcell-finalize"/m);
});

test('decision D54 (empty runs keep the tip) is recorded, the changelog mentions the service', () => {
  const d = read('docs', 'decisions.md');
  assert.match(d, /^## csr-071-crowdcell \(owner\)/m);
  const sec = d.slice(d.indexOf('## csr-071-crowdcell (owner)'));
  assert.match(sec, /\*\*D54\*\*/);
  assert.match(sec, /empty run/i);
  assert.match(sec, /tip/i);
  assert.match(sec, /tip_shannons/);
  const c = read('CHANGELOG.md');
  assert.match(c, /crowdcell-finalize/);
});

test('the spec of the service is in spec/', () => {
  assert.ok(exists('spec', 'crowdcell-finalize.md'));
  const s = read('spec', 'crowdcell-finalize.md');
  for (const needle of ['Payload', 'u32', 'cells_query', 'empty skeleton', 'since', 'status', 'byte 56', 'exit code']) {
    assert.ok(s.toLowerCase().includes(needle.toLowerCase()), `spec mentions ${needle}`);
  }
});
