// ping-v1-1-runner: the new check-in, challenge handling and monitor list are used, and the spec says so.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const read = (...p) => fs.readFileSync(path.join(ROOT, ...p), 'utf8');
const src = (f) => read('runner', 'src', f);
const calls = (text, name) => text.split('\n').filter((l) => new RegExp(`\\b${name}\\s*\\(`).test(l) && !new RegExp(`fn\\s+${name}\\b`).test(l));

test('protocol.rs includes the acceptance test module', () => {
  assert.match(src('ping/protocol.rs'), /#\[path = "\.\.\/\.\.\/\.\.\/acceptance\/aiworker-ping-v1-1-runner\/ping_v11_tests\.rs"\]/);
});

test('the check-in carries ts and checkin_sig', () => {
  const o = src('ping/outbound.rs');
  assert.ok(calls(o, 'checkin_message').length >= 1);
  assert.match(o, /"checkin_sig"/);
  assert.match(o, /"ts"/);
});

test('the inbound side uses challenge_status and the monitor list', () => {
  const i = src('ping/inbound.rs');
  assert.ok(calls(i, 'challenge_status').length >= 1);
  assert.doesNotMatch(i, /"height above tip"/);
  assert.match(i, /clock_skew/);
  assert.ok(calls(i + src('ping/outbound.rs') + src('attest.rs') + src('monitor/mod.rs'), 'monitors').length >= 2);
  assert.ok(calls(src('attest.rs'), 'explain_error').length >= 1);
});

test('the specs describe v1.1 and one error table', () => {
  const p = read('spec', 'ping-protocol.md');
  assert.match(p, /CSR-CHECKIN-v1/);
  assert.match(p, /clock_skew/);
  assert.match(p, /block_hash_at_height.*empty|empty.*block_hash_at_height/s);
  assert.match(p, /\[\[monitors\]\]/);
  const a = read('spec', 'monitor-public-api.md');
  assert.match(a, /403.*stake_in_use/);
  assert.doesNotMatch(a, /409.*stake_in_use/);
  assert.doesNotMatch(a, /wrong_owner/);
  for (const r of ['spent', 'not_found', 'not_dao', 'not_deposit', 'too_small', 'chain_unavailable', 'wrong_network', 'bad_signature']) {
    assert.match(a, new RegExp(r), r);
  }
});
