// csr-services-detail, phase 2: pure helpers of the detail panel (runner/assets/svc-detail.js).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

const ROOT = process.cwd();
const SRC = fs.readFileSync(path.join(ROOT, 'runner', 'assets', 'svc-detail.js'), 'utf8');
const win = {};
vm.runInNewContext(SRC, { window: win });
const S = win.CSRSvc;

const ID = '0x' + 'e6c32cab'.repeat(8);
const TX = '0x' + 'ab'.repeat(32);
const TX2 = '0x' + 'cd'.repeat(32);
const ADDR = 'ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqf8xjyh4yq8g6gwnnxccc8cx5a5g3k34qqz0wza5';
const EX = 'https://testnet.explorer.nervos.org';

test('the asset is a plain script: no external URLs, no network, no DOM access', () => {
  assert.ok(S, 'window.CSRSvc is defined');
  assert.doesNotMatch(SRC, /https?:\/\/[a-z]/i, 'no hard-coded URL');
  assert.doesNotMatch(SRC, /\b(fetch|XMLHttpRequest|document\.|innerHTML|eval\()/);
});

test('shortId keeps the start and the end of a 32-byte ID and leaves other text alone', () => {
  assert.equal(S.shortId(ID), ID.slice(0, 8) + '…' + ID.slice(-4));
  assert.equal(S.shortId('be86725b'), 'be86725b');
  assert.equal(S.shortId(undefined), '');
});

test('outpointTx and explorer links', () => {
  assert.equal(S.outpointTx(TX + ':3'), TX);
  assert.equal(S.outpointTx(TX.toUpperCase().replace('0X', '0x') + ':0'), TX);
  assert.equal(S.outpointTx('nonsense'), null);
  assert.equal(S.outpointTx(TX), null);
  assert.equal(S.explorerTxUrl(EX, TX + ':1'), EX + '/transaction/' + TX);
  assert.equal(S.explorerTxUrl(EX + '/', TX + ':1'), EX + '/transaction/' + TX);
  assert.equal(S.explorerTxUrl(null, TX + ':1'), null, 'devnet has no explorer');
  assert.equal(S.explorerTxUrl('http://insecure', TX + ':1'), null);
  assert.equal(S.explorerTxUrl(EX, 'bad'), null);
  assert.equal(S.explorerAddressUrl(EX, ADDR), EX + '/address/' + ADDR);
  assert.equal(S.explorerAddressUrl(EX, 'javascript:alert(1)'), null);
  assert.equal(S.explorerAddressUrl(EX, ''), null);
});

test('safeUrl accepts https only', () => {
  assert.equal(S.safeUrl('https://example.org/x'), 'https://example.org/x');
  assert.equal(S.safeUrl('http://example.org'), null);
  assert.equal(S.safeUrl('javascript:alert(1)'), null);
  assert.equal(S.safeUrl(undefined), null);
});

test('displayName tells two services of the same name apart', () => {
  const idB = '0x' + '11'.repeat(32);
  const a = S.displayName({ service_id: ID, name: 'crt' }, { name: 'crt', version: '0.2.0', publisher_address: ADDR });
  const b = S.displayName({ service_id: idB, name: 'crt' }, { name: 'crt', version: '0.2.0', publisher_address: ADDR });
  assert.equal(a.title, 'crt');
  assert.notEqual(a.sub, b.sub);
  assert.ok(a.sub.includes(S.shortId(ID)) && a.sub.includes('0.2.0') && a.sub.includes('by '));
  assert.equal(a.noManifest, false);
});

test('a service without a manifest is flagged', () => {
  const d = S.displayName({ service_id: ID, name: 'be86725b', language: 'onchain' }, null);
  assert.equal(d.noManifest, true);
  assert.ok(d.sub.includes(S.shortId(ID)));
  const added = S.displayName({ service_id: ID, name: 'x', manifest_type_id: '0x' + '07'.repeat(32) }, null);
  assert.equal(added.noManifest, false, 'added from a manifest, catalog row not loaded yet');
});

test('facts: everything the catalog knows, with explorer links per cell', () => {
  const svc = { service_id: ID, name: 'crt', outpoint: TX + ':0', data_cells: [TX2 + ':2'], args: '0102', manifest_outpoint: TX2 + ':0' };
  const item = { name: 'crt', version: '0.2.0', description: 'Shares for CRT', publisher_address: ADDR, url: 'https://example.org/crt',
    source: 'ssh://example.org/crt.git', first_seen_ms: 1700000000000, min_csr: '0.6.0', deprecated: false };
  const rows = S.facts(svc, item, { explorer: EX, fmtDate: () => '14 Nov 2026' });
  const by = (l) => rows.find((r) => r.label === l);
  assert.equal(by('Service ID').value, ID);
  assert.equal(by('Version').value, '0.2.0');
  assert.equal(by('Publisher').href, EX + '/address/' + ADDR);
  assert.equal(by('Code cell').href, EX + '/transaction/' + TX);
  assert.equal(by('Data cell 1').href, EX + '/transaction/' + TX2);
  assert.equal(by('Manifest cell').href, EX + '/transaction/' + TX2);
  assert.equal(by('Args').value, '0x0102');
  assert.equal(by('Project').href, 'https://example.org/crt');
  assert.equal(by('Source').href, null, 'a non-https source is text, not a link');
  assert.equal(by('Source').value, 'ssh://example.org/crt.git');
  assert.equal(by('First seen').value, '14 Nov 2026');
  assert.equal(by('Needs CSR').value, '≥ 0.6.0');
  assert.equal(by('Deprecated'), undefined);
});

test('facts: a service without a manifest still shows its ID and code cell', () => {
  const rows = S.facts({ service_id: ID, name: 'be86725b', outpoint: TX + ':0' }, null, { explorer: EX });
  assert.deepEqual(Array.from(rows.map((r) => r.label)), ['Service ID', 'Code cell']);
  assert.equal(rows[1].href, EX + '/transaction/' + TX);
  const local = S.facts({ service_id: ID, name: 'hello' }, null, { explorer: EX });
  assert.deepEqual(Array.from(local.map((r) => r.label)), ['Service ID']);
});

test('approveWarning', () => {
  assert.equal(S.approveWarning({ service_id: ID }, null), S.NO_MANIFEST_WARNING);
  assert.match(S.NO_MANIFEST_WARNING, /No manifest/);
  assert.match(S.approveWarning({ service_id: ID }, { deprecated: true }), /deprecated/);
  assert.equal(S.approveWarning({ service_id: ID }, { deprecated: false }), null);
  assert.match(S.CLAIM_NOTE, /publisher's own claim/);
});

test('verifyText for each status', () => {
  assert.equal(S.verifyText({ status: 'match' }).cls, 'ok');
  assert.equal(S.verifyText({ status: 'mismatch' }).cls, 'bad');
  assert.match(S.verifyText({ status: 'mismatch' }).text, /Do not approve/);
  assert.equal(S.verifyText({ status: 'not_recorded' }).cls, 'warn');
  const u = S.verifyText({ status: 'unavailable', checks: [{ what: 'code cell', ok: false, detail: 'cannot read 0x…: rpc timeout' }] });
  assert.equal(u.cls, 'warn');
  assert.match(u.text, /rpc timeout/);
  assert.equal(S.verifyText(null).cls, 'warn');
});
