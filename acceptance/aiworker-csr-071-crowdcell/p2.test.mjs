// csr-071-crowdcell, phase 2: the runner reports its approved service IDs at check-in.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');

test('outbound check-in sends approved_services, computed by approved_service_ids', () => {
  const o = read('runner', 'src', 'ping', 'outbound.rs');
  assert.match(o, /fn approved_service_ids\(/);
  assert.match(o, /"approved_services"\s*:/);
  // the existing fields stay
  for (const f of ['"peer_id"', '"service_pubkey"', '"binding_sig"', '"network"', '"csr_version"', '"ts"', '"checkin_sig"']) {
    assert.ok(o.includes(f), `body still has ${f}`);
  }
  assert.ok(o.includes('#[path = "../../../acceptance/aiworker-csr-071-crowdcell/outbound_tests.rs"]'));
  assert.ok(o.includes('mod aiworker_071_outbound_tests;'));
});

test('the check-in signature does not cover the list (it is advisory) and the docs say so', () => {
  const p = read('runner', 'src', 'ping', 'protocol.rs');
  assert.ok(!/approved/i.test(p), 'protocol.rs (the signed messages) is unchanged');
  const sm = read('docs', 'security-model.md');
  assert.match(sm, /approved service/i, 'security-model.md lists the approved service IDs among what leaves the machine');
  assert.match(sm, /advisory|not signed|unsigned/i);
  const d = read('docs', 'decisions.md');
  assert.match(d, /\*\*D55\*\*/);
  assert.match(d, /approved_services/);
});

test('the list is only a hint: no service ID is invented and nothing else about services is sent', () => {
  const o = read('runner', 'src', 'ping', 'outbound.rs');
  const body = o.slice(o.indexOf('let body = json!'), o.indexOf('let resp = http'));
  for (const banned of ['"payload"', '"args"', '"outpoint"', '"data_cells"', '"name"']) {
    assert.ok(!body.includes(banned), `check-in body does not contain ${banned}`);
  }
});
