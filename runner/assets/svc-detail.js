// Pure helpers of the Services page detail panel (open item #21). No DOM access, no network, no external URLs:
// explorer links are built from the base URL the runner reports in /api/status (`links.explorer`).
(function () {
  'use strict';
  var ID_RE = /^0x[0-9a-f]{64}$/i;
  var OP_RE = /^(0x[0-9a-f]{64}):(\d+)$/i;
  var ADDR_RE = /^ck[bt]1[0-9a-z]+$/;

  var CLAIM_NOTE = 'The project and source links are the publisher\'s own claim. CSR does not check that the source ' +
    'produces this binary. Reproducible builds would be a later step.';
  var NO_MANIFEST_WARNING = 'No manifest: nothing on chain says who published this code or what it does. ' +
    'Approve it only if you built or checked the binary yourself.';

  function shortId(id) {
    return typeof id === 'string' && ID_RE.test(id) ? id.slice(0, 8) + '…' + id.slice(-4) : (id || '');
  }
  function shortAddress(a) {
    return typeof a === 'string' && a.length > 20 ? a.slice(0, 10) + '…' + a.slice(-6) : (a || '');
  }
  function outpointTx(op) {
    var m = typeof op === 'string' ? OP_RE.exec(op) : null;
    return m ? m[1].toLowerCase() : null;
  }
  function base(b) { return typeof b === 'string' && /^https:\/\//.test(b) ? b.replace(/\/+$/, '') : null; }
  function explorerTxUrl(b, op) {
    var tx = outpointTx(op), root = base(b);
    return tx && root ? root + '/transaction/' + tx : null;
  }
  function explorerAddressUrl(b, addr) {
    var root = base(b);
    return root && typeof addr === 'string' && ADDR_RE.test(addr) ? root + '/address/' + addr : null;
  }
  function safeUrl(u) { return typeof u === 'string' && u.indexOf('https://') === 0 ? u : null; }

  // svc: a row of GET /api/services; item: the matching catalog item (GET /api/catalog) or null.
  function displayName(svc, item) {
    var id = svc.service_id || svc.code_hash || '';
    var noManifest = !item && !svc.manifest_type_id;
    var title = (item && item.name) || svc.name || shortId(id);
    var parts = [];
    if (item && item.version) parts.push(item.version);
    if (item && item.publisher_address) parts.push('by ' + shortAddress(item.publisher_address));
    parts.push(shortId(id));
    return { title: title, sub: parts.join(' · '), noManifest: noManifest };
  }

  function facts(svc, item, opts) {
    opts = opts || {};
    var ex = opts.explorer, fmt = opts.fmtDate || function (ms) { return new Date(ms).toISOString(); };
    var rows = [];
    function add(label, value, href, mono) {
      if (value === undefined || value === null || value === '') return;
      rows.push({ label: label, value: String(value), href: href || null, mono: !!mono });
    }
    var id = svc.service_id || svc.code_hash;
    add('Service ID', id, null, true);
    if (item) {
      add('Name', item.name);
      add('Version', item.version);
      add('Description', item.description);
      add('Publisher', item.publisher_address, explorerAddressUrl(ex, item.publisher_address), true);
      add('Project', item.url, safeUrl(item.url));
      add('Source', item.source, safeUrl(item.source));
      if (item.first_seen_ms) add('First seen', fmt(item.first_seen_ms));
      add('Needs CSR', item.min_csr && item.min_csr !== '0.0.0' ? '≥ ' + item.min_csr : '');
      if (item.deprecated) add('Deprecated', 'yes, by its publisher');
    }
    var code = svc.outpoint || (item && item.code_cell);
    add('Code cell', code, explorerTxUrl(ex, code), true);
    (svc.data_cells || (item && item.data_cells) || []).forEach(function (d, i) {
      add('Data cell ' + (i + 1), d, explorerTxUrl(ex, d), true);
    });
    var args = svc.args || (item && item.args);
    add('Args', args ? '0x' + args : '', null, true);
    var mop = svc.manifest_outpoint || (item && item.outpoint);
    add('Manifest cell', mop, explorerTxUrl(ex, mop), true);
    return rows;
  }

  function approveWarning(svc, item) {
    if (!item && !svc.manifest_type_id) return NO_MANIFEST_WARNING;
    if (item && item.deprecated) return 'The publisher has marked this service as deprecated.';
    return null;
  }

  // report: the body of GET /api/services/{id}/verify.
  function verifyText(report) {
    var s = report && report.status;
    if (s === 'match') return { cls: 'ok', text: 'Matches: the code on chain hashes to this service ID.' };
    if (s === 'mismatch') return { cls: 'bad', text: 'DOES NOT MATCH: the cells on chain do not hash to this service ID. Do not approve.' };
    if (s === 'not_recorded') return { cls: 'warn', text: 'No code cell is recorded for this service (added from a local manifest). Nothing to compare with.' };
    var bad = ((report && report.checks) || []).filter(function (c) { return !c.ok; })[0];
    return { cls: 'warn', text: 'Could not verify: ' + (bad ? bad.detail : 'unknown reason') };
  }

  window.CSRSvc = {
    CLAIM_NOTE: CLAIM_NOTE, NO_MANIFEST_WARNING: NO_MANIFEST_WARNING,
    shortId: shortId, shortAddress: shortAddress, outpointTx: outpointTx,
    explorerTxUrl: explorerTxUrl, explorerAddressUrl: explorerAddressUrl, safeUrl: safeUrl,
    displayName: displayName, facts: facts, approveWarning: approveWarning, verifyText: verifyText
  };
})();
