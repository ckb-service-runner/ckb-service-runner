(function () {
  'use strict';
  var NS = 'http://www.w3.org/2000/svg';
  var view = document.getElementById('view');
  var state = { status: null, csrf: '', auth: true, pwReq: false };
  var timers = [];
  var es = null;

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined && text !== null) e.textContent = text;
    return e;
  }
  function card(title) {
    var c = el('section', 'card');
    c.appendChild(el('h2', '', title));
    return c;
  }
  function dl(c, pairs) {
    var d = el('dl');
    pairs.forEach(function (p) {
      d.appendChild(el('dt', '', p[0]));
      var dd = el('dd');
      if (p[1] instanceof Node) dd.appendChild(p[1]);
      else dd.textContent = p[1] === null || p[1] === undefined ? '–' : String(p[1]);
      d.appendChild(dd);
    });
    c.appendChild(d);
  }
  function pill(v) {
    return el('span', 'pill ' + v, v);
  }
  function api(path, opts) {
    opts = opts || {};
    var h = { Accept: 'application/json' };
    var o = { method: opts.method || 'GET', headers: h, credentials: 'same-origin' };
    if (opts.body !== undefined) {
      h['Content-Type'] = 'application/json';
      h['X-CSRF-Token'] = state.csrf;
      o.body = JSON.stringify(opts.body);
    }
    return fetch(path, o).then(function (r) {
      if (r.status === 401) {
        state.auth = false;
        route();
        throw new Error('authentication required');
      }
      return r.json().then(function (j) {
        if (!r.ok && j.error === 'password_required') {
          pwNotice(j.detail);
          throw new Error(j.detail || 'a dashboard password is required');
        }
        if (!r.ok && j.error === 'mainnet_locked') throw new Error(j.detail || 'Not available on mainnet yet.');
        if (!r.ok) throw new Error(j.error === 'busy' && j.detail ? 'The runner wallet is busy: ' + j.detail + '. Try again after the next block.' : j.error || 'HTTP ' + r.status);
        return j;
      });
    });
  }
  // Banner at the top of the page: wallet, registration, service and settings writes need a password.
  function pwNotice(detail) {
    var old = document.getElementById('pw-notice');
    if (old) old.remove();
    var n = el('div', 'msg err', (detail || 'Set a dashboard password (csr set-password) to use this action.') + ' ');
    n.id = 'pw-notice';
    var a = el('a', '', 'How to set a dashboard password');
    a.href = '#/settings';
    a.addEventListener('click', function () { setTimeout(function () { var t = document.getElementById('password-help'); if (t) t.scrollIntoView(); }, 0); });
    n.appendChild(a);
    view.insertBefore(n, view.firstChild);
  }
  function fmtUp(s) {
    var d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
    return (d ? d + 'd ' : '') + h + 'h ' + m + 'm';
  }

  function chart(samples, key, label) {
    var w = 400, h = 140, p = 24;
    var svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('class', 'chart');
    svg.setAttribute('viewBox', '0 0 ' + w + ' ' + h);
    svg.setAttribute('role', 'img');
    svg.setAttribute('aria-label', label);
    var t = document.createElementNS(NS, 'text');
    t.setAttribute('x', 4); t.setAttribute('y', 10);
    t.textContent = label;
    svg.appendChild(t);
    if (samples.length < 2) {
      var n = document.createElementNS(NS, 'text');
      n.setAttribute('x', w / 2 - 30); n.setAttribute('y', h / 2);
      n.textContent = 'no data yet';
      svg.appendChild(n);
      return svg;
    }
    var t0 = samples[0].t, t1 = samples[samples.length - 1].t || t0 + 1;
    var max = 1;
    samples.forEach(function (s) { if (s[key] > max) max = s[key]; });
    var pts = samples.map(function (s) {
      var x = p + ((s.t - t0) / Math.max(1, t1 - t0)) * (w - p - 4);
      var y = h - 14 - (s[key] / max) * (h - 30);
      return x.toFixed(1) + ',' + y.toFixed(1);
    });
    var ax = document.createElementNS(NS, 'line');
    ax.setAttribute('class', 'axis');
    ax.setAttribute('x1', p); ax.setAttribute('x2', w - 4);
    ax.setAttribute('y1', h - 14); ax.setAttribute('y2', h - 14);
    svg.appendChild(ax);
    var mx = document.createElementNS(NS, 'text');
    mx.setAttribute('x', 2); mx.setAttribute('y', 24);
    mx.textContent = String(Math.round(max));
    svg.appendChild(mx);
    var pl = document.createElementNS(NS, 'polyline');
    pl.setAttribute('class', 'line');
    pl.setAttribute('points', pts.join(' '));
    svg.appendChild(pl);
    return svg;
  }

  function renderLogin() {
    view.textContent = '';
    var c = card('Login');
    var lg = el('img', 'logo-full'); lg.src = '/logo-full.png'; lg.alt = 'ckb-service-runner';
    c.appendChild(lg);
    var f = el('form');
    var i = el('input');
    i.type = 'password'; i.autocomplete = 'current-password'; i.placeholder = 'Password';
    var b = el('button', 'btn primary', 'Log in');
    b.type = 'submit';
    var m = el('div', 'msg err');
    f.appendChild(i); f.appendChild(b); f.appendChild(m);
    f.addEventListener('submit', function (e) {
      e.preventDefault();
      api('/api/login', { method: 'POST', body: { password: i.value } }).then(function (j) {
        state.csrf = j.csrf_token; state.auth = true; i.value = '';
        connect();
        route();
      }).catch(function (x) { m.textContent = x.message; });
    });
    c.appendChild(f);
    view.appendChild(c);
  }

  function fmtTime(t) { return t ? new Date(t * 1000).toLocaleString() : 'never'; }
  function rankText(n) {
    if (typeof n.rank !== 'number') return 'not ranked yet' + (n.provisional ? ' (provisional)' : '');
    return n.rank + ' / ' + n.of + (n.provisional ? ' (provisional)' : '');
  }
  function pct(v) { return typeof v === 'number' ? (v * 100).toFixed(2) + '%' : '–'; }
  function clear(c) { while (c.children.length > 1) c.removeChild(c.lastChild); }
  function stale(c, j) {
    if (j.stale) c.appendChild(el('p', 'msg stale', 'Stale data: showing the last cached monitor reply. ' + (j.error || '')));
    else if (j.error) c.appendChild(el('p', 'msg err', j.error));
  }
  var CHECK_TYPES = {
    p2p_verified: 'P2P-verified: passed the monitor\'s daily full-node check',
    peer_connected: 'Peer connected: your node is a peer of the monitor\'s node (not yet verified)',
    challenge_only: 'Challenge only: the ping was answered, no P2P check',
    none: 'None: the monitor has not checked this node'
  };
  function checkType(t) { return t == null ? null : (CHECK_TYPES[t] || t); }
  function identityProven(n) { return n.identity_proven != null ? !!n.identity_proven : n.check_type === 'p2p_verified'; }
  function countryBars(list) {
    var rows = (list || []).slice().sort(function (a, b) { return b.count - a.count || (a.code < b.code ? -1 : 1); });
    var h = 16, w = 400, lw = 34, max = rows.length ? rows[0].count : 1;
    var svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('class', 'chart bars');
    svg.setAttribute('viewBox', '0 0 ' + w + ' ' + Math.max(h, rows.length * h));
    svg.setAttribute('role', 'img');
    svg.setAttribute('aria-label', 'nodes per country');
    rows.forEach(function (r, i) {
      var y = i * h;
      var t = document.createElementNS(NS, 'text');
      t.setAttribute('x', 2); t.setAttribute('y', y + 12);
      t.textContent = r.code;
      svg.appendChild(t);
      var b = document.createElementNS(NS, 'rect');
      b.setAttribute('class', 'bar');
      b.setAttribute('x', lw); b.setAttribute('y', y + 2); b.setAttribute('height', h - 4);
      b.setAttribute('width', Math.max(1, (r.count / max) * (w - lw - 40)).toFixed(1));
      svg.appendChild(b);
      var n = document.createElementNS(NS, 'text');
      n.setAttribute('x', lw + Math.max(1, (r.count / max) * (w - lw - 40)) + 4); n.setAttribute('y', y + 12);
      n.textContent = String(r.count);
      svg.appendChild(n);
    });
    return svg;
  }

  var pages = {};

  // ---- shared pieces of the v0.4.0 pages ----
  function fmtInt(n) { return typeof n === 'number' ? n.toLocaleString('en-US') : '–'; }
  function fmtAgo(ms) {
    if (typeof ms !== 'number') return '–';
    var s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 60) return s + 's';
    if (s < 3600) return Math.floor(s / 60) + 'm';
    return Math.floor(s / 3600) + 'h ' + Math.floor((s % 3600) / 60) + 'm';
  }
  function fmtLeft(ms) {
    var s = Math.round((ms - Date.now()) / 1000);
    if (s <= 0) return 'expired';
    var d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600);
    return d ? d + ' d ' + h + ' h' : h + ' h ' + Math.floor((s % 3600) / 60) + ' m';
  }
  function fmtDate(ms) {
    return typeof ms === 'number' ? new Date(ms).toLocaleString([], { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }) : '–';
  }
  function ckbShort(sh) {
    if (typeof sh !== 'number') return '–';
    return (sh / 1e8).toLocaleString('en-US', { maximumFractionDigits: 2 });
  }
  function head(c, title, badge, cls) {
    var h = el('div', 'card-head');
    h.appendChild(el('h2', '', title));
    if (badge) h.appendChild(el('span', 'pill ' + (cls || ''), badge));
    c.appendChild(h);
    return h;
  }
  function tile(label, value, unit, cls) {
    var t = el('div', 'tile');
    t.appendChild(el('p', 'lbl', label));
    var b = el('p', 'big' + (cls ? ' ' + cls : ''), value);
    if (unit) { b.appendChild(document.createTextNode(' ')); b.appendChild(el('small', '', unit)); }
    t.appendChild(b);
    return t;
  }
  function kv(c, pairs) {
    var box = el('div');
    pairs.forEach(function (p) {
      var r = el('div', 'kv');
      r.appendChild(el('span', '', p[0]));
      var v = el('span', p[2] || '');
      if (p[1] instanceof Node) v.appendChild(p[1]); else v.textContent = p[1] === null || p[1] === undefined ? '–' : String(p[1]);
      r.appendChild(v);
      box.appendChild(r);
    });
    c.appendChild(box);
    return box;
  }
  function bar(fraction, cls) {
    var b = el('div', 'bar' + (cls ? ' ' + cls : ''));
    var s = el('span');
    s.style.width = Math.max(0, Math.min(100, fraction * 100)).toFixed(1) + '%';
    b.appendChild(s);
    return b;
  }
  function metric(c, label, value, fraction, cls) {
    var m = el('div', 'metric');
    var r = el('div', 'mrow');
    r.appendChild(el('span', '', label));
    r.appendChild(el('span', 'mono', value));
    m.appendChild(r);
    m.appendChild(bar(fraction, cls));
    c.appendChild(m);
  }
  // Mainnet before 1.x (0.5.1): the notice shown instead of registration, wallet funding, jobs and earnings.
  // `msg` is `registration_locked` of /api/status or `message` of a locked /api/registry reply.
  function lockedNotice(msg, gate) {
    var c = el('section', 'card locked-notice');
    head(c, 'Mainnet: dashboard and ranking only', 'COMES WITH 1.x', 'warn');
    c.appendChild(el('p', '', String(msg).replace(/^Mainnet: node dashboard and ranking only\.\s*/, '')));
    if (gate) gateMetrics(c, gate);
    return c;
  }
  // Progress of the Phase 2 gate (monitor `GET /v1/{network}/summary` → `gate`).
  function gateMetrics(c, g) {
    metric(c, 'Ranked runners (' + g.min_runners + ' needed)', fmtInt(g.runners) + ' / ' + fmtInt(g.min_runners), g.runners / Math.max(1, g.min_runners));
    metric(c, 'Monitor history (' + g.min_days + ' days needed)', fmtInt(g.history_days) + ' / ' + fmtInt(g.min_days) + ' days', g.history_days / Math.max(1, g.min_days));
    if (typeof g.max_asn_share === 'number') {
      c.appendChild(el('p', 'note', 'Also needed: no network (ASN) above ' + Math.round(g.max_asn_share * 100) + ' % and no country above ' +
        Math.round(g.max_country_share * 100) + ' % of the ranked runners. Now: ' + Math.round(g.largest_asn_share * 100) + ' % and ' +
        Math.round(g.largest_country_share * 100) + ' %.'));
    }
  }
  function lockedMsg() { return state.status && state.status.registration_locked; }
  // Puts the mainnet notice at the top of `root` once the status is known (also on a first page load).
  function lockedTop(root) {
    function put(m) { if (m) root.insertBefore(lockedNotice(m), root.firstChild); }
    if (state.status) put(lockedMsg());
    else api('/api/status').then(function (st) { put(st.registration_locked); }).catch(function () {});
  }

  // A 24 h sparkline of one sample field; `now` is the value shown at the right.
  function spark(samples, key, label, now) {
    var row = el('div', 'spark');
    row.appendChild(el('span', '', label));
    var pts = samples.filter(function (s) { return typeof s[key] === 'number'; });
    var svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('viewBox', '0 0 240 40');
    svg.setAttribute('preserveAspectRatio', 'none');
    svg.setAttribute('role', 'img');
    if (pts.length >= 2) {
      var t0 = pts[0].t, t1 = pts[pts.length - 1].t, lo = Infinity, hi = -Infinity;
      pts.forEach(function (s) { lo = Math.min(lo, s[key]); hi = Math.max(hi, s[key]); });
      var span = hi - lo || 1;
      var line = pts.map(function (s) {
        return ((s.t - t0) / Math.max(1, t1 - t0) * 240).toFixed(1) + ',' + (36 - (s[key] - lo) / span * 30).toFixed(1);
      }).join(' ');
      var area = document.createElementNS(NS, 'polyline');
      area.setAttribute('class', 'area'); area.setAttribute('points', '0,40 ' + line + ' 240,40');
      var pl = document.createElementNS(NS, 'polyline');
      pl.setAttribute('class', 'line'); pl.setAttribute('points', line);
      svg.appendChild(area); svg.appendChild(pl);
      svg.setAttribute('aria-label', label + ', last 24 hours, from ' + Math.round(lo) + ' to ' + Math.round(hi));
    } else {
      svg.setAttribute('aria-label', label + ': no data yet');
    }
    row.appendChild(svg);
    row.appendChild(el('span', '', now));
    return row;
  }
  function links() { return (state.status && state.status.links) || {}; }
  function linkBtn(text, href) {
    var a = el('a', 'btn', text + ' ↗'); a.href = href; a.target = '_blank'; a.rel = 'noopener noreferrer';
    return a;
  }
  // The CKB node card shared by Overview and Node: tiles, sync bar, facts.
  function nodeHero(c, n, s, full) {
    c.textContent = '';
    var net = (s && s.network) || '';
    var synced = n.tip_lag === 0 || (typeof n.tip_lag === 'number' && n.tip_lag <= 2);
    var r2 = s && s.r2 ? s.r2.state : (synced ? 'Synced' : 'Syncing');
    var hh = el('div', 'hero-head');
    var left = el('div');
    left.appendChild(el('p', 'eyebrow', 'CKB full node'));
    left.appendChild(el('h2', '', 'CKB ' + (net ? net.charAt(0).toUpperCase() + net.slice(1) : 'node')));
    left.appendChild(el('p', 'hero-sub', [n.node_type || 'ckb', n.node_version, n.rpc_url].filter(Boolean).join(' · ')));
    hh.appendChild(left);
    hh.appendChild(el('span', 'pill ' + r2, r2.toUpperCase()));
    c.appendChild(hh);
    var tiles = el('div', 'tiles');
    var tipT = tile('Tip block', fmtInt(n.tip_number));
    var ex = links().explorer;
    if (typeof n.tip_number === 'number' && ex) {
      var b = tipT.querySelector('.big'); b.textContent = '';
      var a = el('a', '', fmtInt(n.tip_number)); a.href = ex + '/block/' + n.tip_number; a.target = '_blank'; a.rel = 'noopener noreferrer';
      b.appendChild(a);
    }
    tiles.appendChild(tipT);
    tiles.appendChild(tile('Behind', typeof n.tip_lag === 'number' ? fmtInt(n.tip_lag) : '–', 'blocks', n.tip_lag === 0 ? 'good' : ''));
    tiles.appendChild(tile('Tip age', fmtAgo(n.tip_timestamp_ms)));
    var peers = (n.peers_in || 0) + (n.peers_out || 0);
    tiles.appendChild(tile('Peers', n.peers_in == null ? '–' : String(peers), n.peers_in == null ? '' : n.peers_in + '↓ ' + n.peers_out + '↑'));
    c.appendChild(tiles);
    var frac = typeof n.best_known === 'number' && n.best_known > 0 && typeof n.tip_number === 'number' ? n.tip_number / n.best_known : 0;
    c.appendChild(bar(frac, synced ? '' : 'warn'));
    var f = el('div', 'facts');
    function fact(label, value) { var sp = el('span', '', label + ' '); sp.appendChild(el('b', '', value)); f.appendChild(sp); }
    if (typeof n.epoch === 'number') fact('Epoch', fmtInt(n.epoch) + (typeof n.epoch_index === 'number' ? ' · ' + fmtInt(n.epoch_index) + '/' + fmtInt(n.epoch_length) : ''));
    fact('Best known', fmtInt(n.best_known));
    fact('Indexer lag', n.indexer_lag == null ? '–' : fmtInt(n.indexer_lag));
    if (n.tx_pool && n.tx_pool.pending != null) fact('Tx pool', n.tx_pool.pending + ' pending · ' + n.tx_pool.proposed + ' proposed');
    var idn = n.identity && n.identity.state;
    if (idn) fact('Node identity', idn);
    c.appendChild(f);
    if (full) {
      var g = el('div', 'facts mono');
      if (n.tip_hash) g.appendChild(el('span', '', 'Tip hash ' + n.tip_hash));
      if (n.node_id) g.appendChild(el('span', '', 'Node ID ' + n.node_id));
      c.appendChild(g);
    }
    if (n.identity && n.identity.message && idn !== 'proven') c.appendChild(el('p', idn === 'unknown' ? 'msg' : 'msg err', n.identity.message));
    if (n.net_rpc_hint) c.appendChild(el('p', 'msg err', n.net_rpc_hint));
    var lk = el('div', 'links'), L = links();
    if (L.explorer) lk.appendChild(linkBtn('Explorer', L.explorer + (typeof n.tip_number === 'number' ? '/block/' + n.tip_number : '')));
    if (L.nodes_list) lk.appendChild(linkBtn('nodes.ckb.dev', L.nodes_list));
    if (lk.children.length) c.appendChild(lk);
  }

  pages.overview = function (root) {
    var heroC = el('section', 'card accent hero');
    var g = el('div', 'grid');
    var ck = card('CSR monitor'), cw = el('section', 'card'), cr = el('section', 'card');
    var ch = el('section', 'card'), cres = el('section', 'card');
    var cn = el('section', 'card'), cu = card('Update');
    var cx = el('section', 'card'); cx.hidden = true;
    [ck, cw, cr].forEach(function (c) { g.appendChild(c); });
    var g2 = el('div', 'grid'); [ch, cres].forEach(function (c) { g2.appendChild(c); });
    var g3 = el('div', 'grid'); [cn, cu].forEach(function (c) { g3.appendChild(c); });
    [cx, heroC, g, g2, g3].forEach(function (x) { root.appendChild(x); });
    var upd = updateCard(cu);
    var last = { node: null };

    function paintStatus() {
      var s = state.status;
      if (!s) return;
      upd.notice(s.update);
      if (last.node) nodeHero(heroC, last.node, s, false);
    }
    function loadNode() {
      api('/api/node').then(function (n) { last.node = n; nodeHero(heroC, n, state.status, false); }).catch(function () {});
    }
    function loadMonitor() {
      api('/api/network').then(function (j) {
        ck.textContent = '';
        var n = j.node;
        var CT = { p2p_verified: 'P2P VERIFIED', peer_connected: 'PEER CONNECTED', challenge_only: 'CHALLENGE ONLY' };
        var hd = head(ck, 'CSR monitor', n ? (CT[n.check_type] || null) : null, n && n.check_type === 'p2p_verified' ? 'ok' : '');
        if (n && hd.lastChild && hd.lastChild.title !== undefined) hd.lastChild.title = checkType(n.check_type) || '';
        if (!j.join) {
          ck.appendChild(el('p', 'note', j.message || 'Join the CSR monitor to see stats'));
          cn.textContent = '';
          head(cn, 'CSR network');
          cn.appendChild(el('p', 'note', 'Join the CSR monitor (Settings) to see the network, your rank and your score. Being ranked is needed to get jobs.'));
          return;
        }
        stale(ck, j);
        var big = el('div', 'row');
        var rank = el('p', 'big xl good', n && typeof n.rank === 'number' ? '#' + n.rank : '–');
        big.appendChild(rank);
        var sub = el('p', 'note', n && typeof n.rank === 'number' ? 'of ' + n.of + ' ranked runners' : (n && n.provisional ? 'provisional: ranked after 7 days' : 'not ranked yet'));
        if (n && typeof n.score === 'number') { sub.appendChild(el('br')); sub.appendChild(document.createTextNode('score ')); sub.appendChild(el('b', 'mono', n.score.toFixed(1))); sub.appendChild(document.createTextNode(' / 100')); }
        big.appendChild(sub);
        ck.appendChild(big);
        var m = n && n.metrics;
        if (m) {
          metric(ck, 'Availability, 30 days', pct(m.a30), m.a30);
          metric(ck, 'In sync, 30 days', pct(m.s30), m.s30);
          metric(ck, 'P2P checks, 30 days', pct(m.f30), m.f30);
          metric(ck, 'Age (full at 1 year)', m.age_days + ' days', m.t);
        } else if (n) {
          metric(ck, 'Availability, 30 days', pct(n.availability.d30), n.availability.d30 || 0);
        }
        var ol = el('a', '', 'Open the CSR monitor →'); ol.href = '#/network'; ck.appendChild(ol);
        // network card
        cn.textContent = '';
        head(cn, 'CSR network');
        var nw = j.network;
        var tl = el('div', 'tiles sep');
        tl.appendChild(tile('Runners', nw ? fmtInt(nw.nodes_total) : '–'));
        tl.appendChild(tile('Reachable', nw ? fmtInt(nw.nodes_reachable) : '–'));
        tl.appendChild(tile('Countries', nw && nw.countries ? String(nw.countries.length) : '–'));
        cn.appendChild(tl);
        var p = el('p', 'note', 'Monitor ' + (j.monitor_url || '–') + ' · last successful check ' + ((j.ping && j.ping.last_check_ok_at) ? fmtAgo(j.ping.last_check_ok_at * 1000) + ' ago' : 'never') + '.');
        p.style.marginTop = '.7rem';
        cn.appendChild(p);
        if (j.monitor_url) { var lk = el('div', 'links'); lk.style.marginTop = '.7rem'; lk.appendChild(linkBtn('CSR monitor', j.monitor_url)); cn.appendChild(lk); }
        if (j.gate && lockedMsg()) {
          var gh = el('h3', '', 'Until services start on mainnet (Phase 2)'); gh.style.marginTop = '1rem';
          cn.appendChild(gh);
          gateMetrics(cn, j.gate);
        }
      }).catch(function () {});
    }
    function loadWallet() {
      var werr = null;
      Promise.all([api('/api/wallet').catch(function (x) { werr = x.message; return {}; }), api('/api/registry').catch(function () { return null; }),
        api('/api/earnings').catch(function () { return null; }), api('/api/jobs').catch(function () { return null; })]).then(function (r) {
        var w = r[0], reg = r[1], e = r[2], jobs = (r[3] && r[3].jobs) || [];
        cw.textContent = '';
        if (reg && reg.locked) {
          head(cw, 'Wallet & registration', 'MAINNET 1.x', 'warn');
          cw.appendChild(el('p', 'note', reg.message));
          if (w && typeof w.capacity_shannons === 'number' && w.capacity_shannons > 0) {
            kv(cw, [['Runner wallet', ckbShort(w.capacity_shannons) + ' CKB (send it back on the Wallet page)', 'mono']]);
          }
          var la = el('a', '', 'Wallet & registration →'); la.href = '#/wallet'; cw.appendChild(la);
          cx.hidden = true;
          paintRuntime();
          return;
        }
        var auto = reg && reg.auto_renew !== false;
        head(cw, 'Wallet & registration', reg && reg.registered ? (auto ? 'AUTO RENEW ON' : 'AUTO RENEW OFF') : 'NOT REGISTERED', reg && reg.registered ? (auto ? 'ok' : 'warn') : 'warn');
        var bal = el('div');
        bal.appendChild(el('p', 'lbl', 'Runner wallet'));
        var b = el('p', 'big lg', ckbShort(w.capacity_shannons)); b.appendChild(document.createTextNode(' ')); b.appendChild(el('small', '', 'CKB'));
        bal.appendChild(b);
        cw.appendChild(bal);
        if (werr) cw.appendChild(el('p', 'msg err', 'Wallet unavailable: ' + werr));
        if (w.sweep_warning) cw.appendChild(el('p', 'msg err', w.sweep_warning));
        var rows = [];
        if (reg && reg.registered) {
          var st = reg.stake;
          rows.push(['Stake', st ? ckbShort(st.capacity_shannons) + ' CKB · ' + st.status : '–', 'mono']);
          rows.push(['Attestation valid until', fmtDate(reg.attestation_expiry_ms) + ' (' + fmtLeft(reg.attestation_expiry_ms) + ')']);
          rows.push([auto ? 'Next renewal' : 'Auto renew', auto ? fmtDate(reg.next_renewal_ms) + ', automatic' : 'off: renew by hand']);
        } else {
          rows.push(['Registration', 'none: register on the Wallet page']);
        }
        if (!werr) rows.push(['Sweep', w.sweep_address ? fmtInt(w.sweep_amount_ckb) + ' CKB when ' + fmtInt(w.sweep_threshold_ckb) + ' CKB' : 'not set']);
        kv(cw, rows);
        if (reg && reg.renewal_error) cw.appendChild(el('p', 'msg err', 'Last renewal error: ' + reg.renewal_error));
        var a = el('a', '', 'Wallet & registration →'); a.href = '#/wallet'; cw.appendChild(a);
        // attention card: expiry close and no automatic renewal, or an error
        var exp = reg && reg.registered ? reg.attestation_expiry_ms : null;
        var close = typeof exp === 'number' && exp - Date.now() < 3 * 86400000;
        if (close && (!auto || reg.renewal_error || exp < Date.now())) {
          cx.textContent = '';
          head(cx, 'Attestation', exp < Date.now() ? 'EXPIRED' : 'EXPIRES SOON', 'bad');
          cx.appendChild(el('p', 'msg err', (exp < Date.now() ? 'The attestation has expired (' : 'The attestation expires on ') + fmtDate(exp) + (exp < Date.now() ? ').' : '.') +
            (auto ? '' : ' Auto renew is off.') + (reg.renewal_error ? ' Last renewal error: ' + reg.renewal_error : '')));
          var wl = el('a', '', 'Wallet & registration →'); wl.href = '#/wallet'; cx.appendChild(wl);
          cx.hidden = false;
        } else {
          cx.hidden = true;
        }
        paintRuntime();
        function paintRuntime() {
        // runtime card
        var s = state.status || {};
        cr.textContent = '';
        var rt = (s.runtime && s.runtime.state) || 'off';
        head(cr, 'Service runtime', rt.toUpperCase(), rt);
        function count(states) { return jobs.filter(function (j) { return states.indexOf(j.state) >= 0; }).length; }
        var tl = el('div', 'tiles sep three');
        tl.appendChild(tile('Running', String(count(['Running', 'Submitting', 'Completed'])), '', 'sm'));
        tl.appendChild(tile('Waiting', String(count(['Discovered', 'Queued', 'Scheduled', 'Paused'])), '', 'sm'));
        tl.appendChild(tile('Settled', String(count(['Settled'])), '', 'sm'));
        cr.appendChild(tl);
        kv(cr, [['Tips earned', e ? ckbShort(e.tips_shannons) + ' CKB' : '–', 'mono'], ['Settlements', e ? String(e.settlements.length) : '–'],
          ['Reason', (s.runtime && s.runtime.reason) || 'running']]);
        var locked = s.network === 'mainnet' && !s.mainnet_runtime_unlocked;
        var tg = el('label', 'runtime-toggle' + (locked ? ' disabled' : ''));
        var cb = document.createElement('input');
        cb.type = 'checkbox'; cb.id = 'runtime-toggle'; cb.checked = rt === 'on' || rt === 'active'; cb.disabled = locked;
        tg.appendChild(cb); tg.appendChild(document.createTextNode(' Service runtime on'));
        cb.addEventListener('change', function () {
          cb.disabled = true;
          api('/api/settings', { method: 'PUT', body: { runtime: { enabled: cb.checked } } })
            .then(function () { poll(); loadWallet(); })
            .catch(function (x) { cb.checked = !cb.checked; cr.appendChild(el('p', 'msg err', x.message)); })
            .then(function () { cb.disabled = locked; });
        });
        cr.appendChild(tg);
        if (!jobs.length) cr.appendChild(el('p', 'note', locked ? 'Services start on mainnet with CSR 1.x (Phase 2).' : 'No jobs yet. The node earns once apps such as CRT post jobs.'));
        }
      }).catch(function () {});
    }
    function loadHistory() {
      Promise.all([api('/api/resources/history'), api('/api/resources')]).then(function (r) {
        var h = r[0].samples || [], now = r[1];
        ch.textContent = '';
        head(ch, 'Last 24 hours');
        var lastLag = h.length ? h[h.length - 1].tip_lag : null;
        ch.appendChild(spark(h, 'tip_lag', 'Tip lag (blocks)', lastLag == null ? '–' : String(lastLag)));
        ch.appendChild(spark(h, 'cpu_percent', 'CPU %', now.cpu.total_percent + '%'));
        ch.appendChild(spark(h, 'ram_free_mb', 'Free RAM (MB)', fmtInt(now.ram.free_mb)));
        cres.textContent = '';
        head(cres, 'Machine');
        var ramUsed = now.ram.total_mb - now.ram.free_mb, diskUsed = now.disk.total_gb - now.disk.free_gb;
        metric(cres, 'CPU (csr ' + now.cpu.csr_percent + '%' + (now.cpu.ckb_percent == null ? '' : ', ckb ' + now.cpu.ckb_percent + '%') + ')', now.cpu.total_percent + '%', now.cpu.total_percent / 100, now.cpu.total_percent > 85 ? 'warn' : '');
        metric(cres, 'RAM used (csr ' + now.ram.csr_mb + ' MB)', (ramUsed / 1024).toFixed(1) + ' / ' + (now.ram.total_mb / 1024).toFixed(1) + ' GB', ramUsed / Math.max(1, now.ram.total_mb));
        metric(cres, 'Disk used', fmtInt(diskUsed) + ' / ' + fmtInt(now.disk.total_gb) + ' GB', diskUsed / Math.max(1, now.disk.total_gb), now.disk.free_gb < 20 ? 'warn' : '');
        var ra = el('a', '', 'Resources →'); ra.href = '#/resources'; cres.appendChild(ra);
      }).catch(function () {});
    }
    onStatus = paintStatus; paintStatus();
    loadNode(); loadMonitor(); loadWallet(); loadHistory();
    timers.push(setInterval(loadNode, 10000));
    timers.push(setInterval(loadWallet, 15000));
    timers.push(setInterval(loadHistory, 30000));
    timers.push(setInterval(loadMonitor, 60000));
  };

  // Update card: daily notice (from /api/status), "Check for update" and, with a newer signed
  // release and a configured release key, "Update" with an in-page confirmation.
  function updateCard(c) {
    var note = el('p', 'msg'), info = el('div'), msg = el('div', 'msg');
    var row = el('div', 'row');
    var check = el('button', 'btn', 'Check for update');
    var go = el('button', 'btn primary', 'Update'); go.hidden = true;
    var sure = el('button', 'btn primary', 'Yes, download and install'); sure.hidden = true;
    var cancel = el('button', 'btn', 'Cancel'); cancel.hidden = true;
    [check, go, sure, cancel].forEach(function (b) { b.type = 'button'; row.appendChild(b); });
    [note, info, row, msg].forEach(function (x) { c.appendChild(x); });
    function say(cls, text) { msg.className = 'msg' + (cls ? ' ' + cls : ''); msg.textContent = text; }
    function link(u) { var a = el('a', '', 'Release notes'); a.href = u; a.target = '_blank'; a.rel = 'noopener noreferrer'; return a; }
    function show(u) {
      info.textContent = '';
      dl(info, [['Current', u.current], ['Latest', u.latest], ['Checked', fmtTime(u.checked_at)], ['Notes', link(u.notes_url)]]);
      var can = u.newer === true && u.key_configured;
      go.hidden = !can; sure.hidden = true; cancel.hidden = true;
      if (u.newer === true && !u.key_configured) say('', 'This build has no release key configured: install the update manually (docs/install.md).');
      else if (u.newer === false) say('ok', 'CSR is up to date.');
      else if (u.newer === null) say('', 'The latest release tag is not a plain version; nothing to install.');
      else say('', '');
    }
    check.addEventListener('click', function () {
      check.disabled = true; msg.className = 'msg'; msg.textContent = 'Asking GitHub for the latest release…';
      api('/api/update').then(show).catch(function (x) { msg.className = 'msg err'; msg.textContent = x.message; })
        .then(function () { check.disabled = false; });
    });
    go.addEventListener('click', function () {
      go.hidden = true; sure.hidden = false; cancel.hidden = false;
      msg.className = 'msg'; msg.textContent = 'Download the release, verify its signature and install it? The runner restarts afterwards.';
    });
    cancel.addEventListener('click', function () { go.hidden = false; sure.hidden = true; cancel.hidden = true; msg.textContent = ''; });
    sure.addEventListener('click', function () {
      sure.disabled = cancel.disabled = check.disabled = true;
      msg.className = 'msg'; msg.textContent = 'Downloading and verifying the release (binary, SHA256SUMS, SHA256SUMS.sig)…';
      api('/api/update/install', { method: 'POST', body: {} }).then(function (r) {
        sure.hidden = cancel.hidden = true;
        msg.className = 'msg ok'; msg.textContent = r.message || 'Update staged; the runner will restart.';
      }).catch(function (x) {
        msg.className = 'msg err'; msg.textContent = 'Update failed: ' + x.message;
        sure.hidden = cancel.hidden = true; go.hidden = false;
      }).then(function () { sure.disabled = cancel.disabled = check.disabled = false; });
    });
    return {
      notice: function (u) {
        if (u && u.newer === true) {
          note.className = 'msg stale'; note.textContent = 'CSR ' + u.latest + ' is available (this is ' + u.current + '). ';
          note.appendChild(link(u.notes_url));
        } else {
          note.className = 'msg'; note.textContent = 'Version ' + ((state.status || {}).version || '–');
        }
      }
    };
  }

  pages.network = function (root) {
    var cw = card('Nodes worldwide'), cr = card('My rank'), cm = card('Map'), cc = card('Countries');
    var g = el('div', 'grid');
    g.appendChild(cw); g.appendChild(cr);
    var mapBox = el('div', 'map'), mapInfo = el('p', '', 'Tap a dot for details.'), dots = [];
    mapInfo.id = 'map-info';
    cm.appendChild(mapBox); cm.appendChild(mapInfo);
    cm.appendChild(el('p', 'credit', 'Map: Natural Earth \u00b7 IP geolocation: DB-IP.com (CC BY 4.0) \u00b7 dots show 2\u00b0 areas, not exact places'));
    mapBox.addEventListener('click', function (ev) {
      var t = ev.target;
      while (t && t !== mapBox && !(t.getAttribute && t.getAttribute('data-index') !== null && t.nodeName.toLowerCase() === 'circle')) t = t.parentNode;
      if (!t || t === mapBox) return;
      var d = dots[Number(t.getAttribute('data-index'))];
      if (d) mapInfo.textContent = d.label;
    });
    function drawMap(nw, own) {
      var ok = !!(nw && nw.locations_available);
      var list = ok ? (nw.locations || []) : [];
      mapBox.innerHTML = window.CSRWorldMap.svg(list, {label: 'CSR runners on a world map', own: own});
      dots = window.CSRWorldMap.dots(list, own);
      if (!ok) { mapInfo.textContent = 'Location data is not available from the monitor.'; return; }
      mapInfo.textContent = 'Tap a dot for details.';
      if (dots.some(function (d) { return d.own; })) {
        mapInfo.appendChild(el('span', 'wm-key'));
        mapInfo.appendChild(document.createTextNode('your node'));
      }
    }
    root.appendChild(g); root.appendChild(cm); root.appendChild(cc);
    function load() {
      api('/api/network').then(function (j) {
        clear(cw); clear(cr); clear(cc);
        if (!j.join) {
          cw.appendChild(el('p', '', j.message || 'Join the CSR monitor to see stats'));
          cm.hidden = true;
          return;
        }
        cm.hidden = false;
        stale(cw, j);
        var nw = j.network, n = j.node;
        drawMap(nw, n && n.location);
        dl(cw, [['Nodes worldwide', nw ? nw.nodes_total : null], ['Nodes reachable', nw ? nw.nodes_reachable : null],
          ['Updated', nw ? fmtTime(nw.updated_at) : null], ['Fetched', fmtTime(j.fetched_at)], ['Monitor', j.monitor_url]]);
        var ping = j.ping || {};
        if (j.warning) cw.insertBefore(el('p', 'msg err', j.warning), cw.children[1] || null);
        if (ping.clock_skew_at) cw.appendChild(el('p', 'msg err', 'Warning: a monitor challenge was outside the 60 s freshness window (last at ' + fmtTime(ping.clock_skew_at) + '). Check this machine\'s clock.'));
        if (n) {
          var a = n.attestation || {};
          dl(cr, [['Rank', rankText(n)], ['Availability 7 d', pct(n.availability.d7)], ['Availability 30 d', pct(n.availability.d30)],
            ['Availability 365 d', pct(n.availability.d365)], ['Check type', checkType(n.check_type)],
            ['Identity proven', identityProven(n) ? 'yes (service key and P2P key bound, full-node check passed)' : 'no'],
            ['Attestation', a.status + (a.expiry ? ' (expires ' + fmtTime(a.expiry) + ')' : '')],
            ['Stake', n.stake ? n.stake.status + (n.stake.capacity_ckb != null ? ' (' + n.stake.capacity_ckb + ' CKB)' : '') : null],
            ['Last check', fmtTime(n.last_check_at)], ['Ping mode', ping.mode], ['Last successful check', fmtTime(ping.last_check_ok_at)]]);
        } else {
          cr.appendChild(el('p', '', nw ? 'Not ranked yet.' : 'No data available.'));
          dl(cr, [['Ping mode', ping.mode], ['Last successful check', fmtTime(ping.last_check_ok_at)]]);
        }
        var idn = j.identity || {};
        var proofs = (idn.proofs || []).map(function (p) {
          return [p.monitor, proofText(p.result) + (p.error ? ' (' + (p.status ? p.status + ' ' : '') + p.error + ')' : '') + ', ' + fmtTime(p.at)];
        });
        dl(cr, [['Node identity', idn.state]].concat(proofs.length ? proofs : [['Identity proof', idn.state === 'proven' ? 'not sent yet' : 'not sent']]));
        if (nw && nw.countries && nw.countries.length) cc.appendChild(countryBars(nw.countries));
        else cc.appendChild(el('p', '', 'No country data.'));
      }).catch(function (x) { clear(cw); cw.appendChild(el('p', 'msg err', x.message)); });
    }
    load();
    timers.push(setInterval(load, 30000));
  };

  function proofText(r) {
    return {accepted: 'accepted', not_supported: 'monitor does not support it yet', rejected: 'rejected', failed: 'failed'}[r] || r;
  }

  pages.node = function (root) {
    var hero = el('section', 'card accent hero');
    var g = el('div', 'grid');
    var ci = el('section', 'card'), chs = el('section', 'card');
    g.appendChild(ci); g.appendChild(chs);
    var cpeer = el('section', 'card');
    [hero, g, cpeer].forEach(function (x) { root.appendChild(x); });
    function load() {
      api('/api/node').then(function (n) {
        nodeHero(hero, n, state.status, true);
        ci.textContent = '';
        var idn = n.identity || {};
        head(ci, 'Node identity', (idn.state || 'unknown').toUpperCase(), idn.state);
        ci.appendChild(el('p', 'note', idn.state === 'proven'
          ? 'The runner holds this node\'s P2P key, so the monitors know the ranked node really is yours.'
          : 'Set node.p2p_key_file (docs/install.md) so the runner can prove that this node is yours.'));
        var rows = [['Node ID', n.node_id || '–', 'mono'], ['State', idn.state || '–']];
        if (idn.message && idn.state !== 'proven') rows.push(['Detail', idn.message]);
        rows.push(['Net RPC (add_node)', n.net_rpc_hint ? 'missing: enable the Net module' : 'available']);
        kv(ci, rows);
        cpeer.textContent = '';
        head(cpeer, 'Peers & tx pool');
        var tl = el('div', 'tiles sep');
        tl.appendChild(tile('Inbound', n.peers_in == null ? '–' : String(n.peers_in), '', 'sm'));
        tl.appendChild(tile('Outbound', n.peers_out == null ? '–' : String(n.peers_out), '', 'sm'));
        tl.appendChild(tile('Pending txs', n.tx_pool && n.tx_pool.pending != null ? String(n.tx_pool.pending) : '–', '', 'sm'));
        tl.appendChild(tile('Proposed txs', n.tx_pool && n.tx_pool.proposed != null ? String(n.tx_pool.proposed) : '–', '', 'sm'));
        tl.appendChild(tile('Indexer tip', fmtInt(n.indexer_tip), '', 'sm'));
        cpeer.appendChild(tl);
      }).catch(function (x) { hero.textContent = ''; hero.appendChild(el('p', 'msg err', x.message)); });
      api('/api/resources/history').then(function (h) {
        var s = h.samples || [];
        chs.textContent = '';
        head(chs, 'Tip lag, last 24 hours');
        chs.appendChild(chart(s, 'tip_lag', 'blocks behind'));
        var lags = s.map(function (x) { return x.tip_lag; }).filter(function (v) { return typeof v === 'number'; });
        var max = lags.length ? Math.max.apply(null, lags) : null;
        var inSync = lags.length ? lags.filter(function (v) { return v <= 2; }).length / lags.length : null;
        kv(chs, [['Most behind', max == null ? '–' : max + ' blocks'], ['In sync (≤ 2 blocks)', inSync == null ? '–' : pct(inSync)]]);
      }).catch(function () {});
    }
    load();
    timers.push(setInterval(load, 10000));
  };

  pages.resources = function (root) {
    var c = card('Now'), c1 = card('CPU % (24 h)'), c2 = card('Free RAM MB (24 h)'), c3 = card('Free disk GB (24 h)');
    root.appendChild(c); root.appendChild(c1); root.appendChild(c2); root.appendChild(c3);
    function load() {
      api('/api/resources').then(function (r) {
        while (c.children.length > 1) c.removeChild(c.lastChild);
        dl(c, [['CPU total', r.cpu.total_percent + '%'], ['CPU csr', r.cpu.csr_percent + '%'], ['CPU ckb', r.cpu.ckb_percent === null ? '–' : r.cpu.ckb_percent + '%'],
          ['RAM free', r.ram.free_mb + ' MB'], ['RAM total', r.ram.total_mb + ' MB'], ['RAM csr', r.ram.csr_mb + ' MB'],
          ['Disk free', r.disk.free_gb + ' GB'], ['Disk total', r.disk.total_gb + ' GB']]);
      }).catch(function () {});
      api('/api/resources/history').then(function (h) {
        while (c1.children.length > 1) c1.removeChild(c1.lastChild);
        while (c2.children.length > 1) c2.removeChild(c2.lastChild);
        c1.appendChild(chart(h.samples, 'cpu_percent', 'CPU %'));
        c2.appendChild(chart(h.samples, 'ram_free_mb', 'free MB'));
        while (c3.children.length > 1) c3.removeChild(c3.lastChild);
        c3.appendChild(chart(h.samples.filter(function (s) { return s.disk_free_gb !== null && s.disk_free_gb !== undefined; }), 'disk_free_gb', 'free GB'));
      }).catch(function () {});
    }
    load();
    timers.push(setInterval(load, 10000));
  };

  var FIELDS = [
    ['node.rpc_url', 'Node RPC URL', 'text', true],
    ['node.node_type', 'Node type', 'select:auto,ckb,neuron'],
    ['dashboard.bind', 'Dashboard bind', 'text', true],
    ['ranking.join', 'Join the CSR monitor', 'bool', true],
    ['ranking.ping_mode', 'Ping mode', 'select:inbound,outbound,both', true],
    ['ranking.ping_bind', 'Ping bind', 'text', true],
    ['ranking.public_endpoint', 'Public endpoint (an https or http URL with host and port, empty = none)', 'text'],
    ['monitor.url', 'Monitor URL', 'text'],
    ['monitor.pubkey', 'Monitor pubkey', 'text'],
    ['payout.sweep_address', 'Sweep to address', 'text'],
    ['payout.sweep_threshold_ckb', 'Sweep when the wallet reaches (CKB)', 'int'],
    ['payout.sweep_amount_ckb', 'Sweep amount (CKB)', 'int'],
    ['registry.auto_renew', 'Auto renew the attestation', 'bool'],
    ['runtime.enabled', 'Runtime enabled', 'bool'],
    ['runtime.min_ckb_per_mcycle', 'Min CKB per Mcycle', 'number'],
    ['limits.max_concurrent_jobs', 'Max concurrent jobs', 'int'],
    ['limits.max_cycles_per_job', 'Max cycles per job', 'int'],
    ['limits.max_job_wall_seconds', 'Max job wall seconds', 'int'],
    ['limits.pause_above_cpu_percent', 'Pause above CPU %', 'int'],
    ['limits.min_free_ram_mb', 'Min free RAM (MB)', 'int'],
    ['limits.min_free_disk_gb', 'Min free disk (GB)', 'int'],
    ['storage.path', 'Storage path', 'text'],
    ['storage.total_quota_mb', 'Total quota (MB)', 'int'],
    ['storage.per_service_quota_mb', 'Per-service quota (MB)', 'int'],
    ['logging.level', 'Log level', 'select:error,warn,info,debug,trace']
  ];
  function getp(o, path) { return path.split('.').reduce(function (a, k) { return a == null ? a : a[k]; }, o); }
  function setp(o, path, v) {
    var ks = path.split('.'), last = ks.pop();
    ks.forEach(function (k) { o = o[k] = o[k] || {}; });
    o[last] = v;
  }

  pages.settings = function (root) {
    var c = card('Settings');
    root.appendChild(c);
    var ph = el('p', '', state.pwReq ? 'A dashboard password is set.' : 'Dashboard password: not set. Run "csr set-password" on the runner host and restart; writes stay refused until then.');
    ph.id = 'password-help';
    c.appendChild(ph);
    api('/api/settings').then(function (cfg) {
      var f = el('form'), inputs = [];
      var ml = cfg.monitors || [];
      if (ml.length) {
        f.appendChild(el('p', '', 'Monitors (edit [[monitors]] in the config file):'));
        var ul = el('ul');
        ml.forEach(function (mo) { ul.appendChild(el('li', '', (mo.url || 'network default') + ' \u00b7 ' + (mo.pubkey || 'network default key'))); });
        f.appendChild(ul);
      }
      FIELDS.forEach(function (fd) {
        var l = el('label', '', fd[1]);
        if (fd[3]) l.appendChild(el('span', 'restart', ' (restart required)'));
        var i, kind = fd[2], v = getp(cfg, fd[0]);
        if (kind.indexOf('select:') === 0) {
          i = el('select');
          kind.slice(7).split(',').forEach(function (o) { var op = el('option', '', o); op.value = o; i.appendChild(op); });
          i.value = v;
        } else if (kind === 'bool') {
          i = el('input'); i.type = 'checkbox'; i.checked = !!v;
        } else {
          i = el('input'); i.type = kind === 'text' ? 'text' : 'number'; i.value = v;
          if (kind === 'number') i.step = 'any';
        }
        l.htmlFor = i.id = 'f-' + fd[0];
        f.appendChild(l); f.appendChild(i);
        inputs.push([fd, i]);
      });
      var allowed = el('input'); allowed.type = 'text'; allowed.id = 'f-allowed';
      allowed.value = (getp(cfg, 'runtime.allowed_services') || []).join(',');
      var al = el('label', '', 'Allowed services (comma separated)'); al.htmlFor = 'f-allowed';
      f.appendChild(al); f.appendChild(allowed);
      var b = el('button', 'btn primary', 'Save'); b.type = 'submit';
      var m = el('div', 'msg');
      f.appendChild(el('p')); f.appendChild(b); f.appendChild(m);
      f.addEventListener('submit', function (e) {
        e.preventDefault();
        var patch = {};
        inputs.forEach(function (x) {
          var fd = x[0], i = x[1], k = fd[2], v;
          if (k === 'bool') v = i.checked;
          else if (k === 'int' || k === 'number') v = Number(i.value);
          else v = i.value;
          setp(patch, fd[0], v);
        });
        setp(patch, 'runtime.allowed_services', allowed.value.split(',').map(function (s) { return s.trim(); }).filter(Boolean));
        api('/api/settings', { method: 'PUT', body: patch }).then(function (r) {
          m.className = 'msg ok';
          m.textContent = 'Saved.' + (r.restart_required ? ' Restart required for some changes to take effect.' : '');
        }).catch(function (x) { m.className = 'msg err'; m.textContent = x.message; });
      });
      c.appendChild(f);
    }).catch(function (x) { c.appendChild(el('p', 'msg err', x.message)); });
  };

  pages.services = function (root) {
    var c = card('Services');
    var msg = el('p', 'msg');
    root.appendChild(c); root.appendChild(msg);
    function act(hash, verb) {
      api('/api/services/' + hash + '/' + verb, { method: 'POST', body: {} }).then(function () {
        msg.className = 'msg'; msg.textContent = '';
        load();
      }).catch(function (x) { msg.className = 'msg err'; msg.textContent = x.message; });
    }
    function load() {
      api('/api/services').then(function (list) {
        clear(c);
        if (!list.length) { c.appendChild(el('p', '', 'No services registered. Use `csr service add-local --manifest PATH`.')); return; }
        var t = el('table');
        var h = el('tr');
        ['Name', 'Language', 'Code hash', 'Status', 'Storage', 'Jobs', 'Earned (shannons)', ''].forEach(function (x) { h.appendChild(el('th', '', x)); });
        t.appendChild(h);
        list.forEach(function (s) {
          var r = el('tr');
          [s.name, s.language, s.code_hash].forEach(function (x) { r.appendChild(el('td', '', x)); });
          var st = el('td'); st.appendChild(pill(s.status)); r.appendChild(st);
          [s.storage_bytes + ' B', s.jobs, s.earned_shannons].forEach(function (x) { r.appendChild(el('td', '', String(x))); });
          var a = el('td');
          var verbs = s.status === 'not_approved' ? ['approve'] : s.status === 'approved_enabled' ? ['disable', 'revoke'] : ['enable', 'revoke'];
          verbs.forEach(function (v) {
            var b = el('button', 'btn', v);
            b.type = 'button';
            b.addEventListener('click', function () { act(s.code_hash, v); });
            a.appendChild(b);
          });
          r.appendChild(a);
          t.appendChild(r);
        });
        c.appendChild(t);
      }).catch(function (x) { clear(c); c.appendChild(el('p', 'msg err', x.message)); });
    }
    load();
  };

  pages.wallet = function (root) {
    var msg = el('p', 'msg');
    function say(t, bad) { msg.className = 'msg' + (bad ? ' err' : ''); msg.textContent = t; }
    function copyBtn(get, box) {
      var b = el('button', 'btn', 'Copy'); b.type = 'button';
      b.addEventListener('click', function () {
        var v = get();
        if (!v) return;
        function fallback() {
          var ok = false;
          try { box.focus(); box.select(); ok = document.execCommand('copy'); } catch (e) { ok = false; }
          if (ok) say('Copied.'); else { try { box.select(); } catch (e) { /* ignore */ } say('Press Ctrl+C to copy', true); }
        }
        if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(v).then(function () { say('Copied.'); }, fallback);
        else fallback();
      });
      return b;
    }
    var c = card('Runner wallet');
    var warn = el('p', 'msg err'); warn.style.display = 'none'; c.appendChild(warn);
    var intro = el('p', '', 'Send about 700 CKB here from your wallet to start. The registration cell uses about 540 CKB, the rest pays small fees. Tips are added here.');
    c.appendChild(intro);
    var info = el('div');
    var addr = el('div', 'addr-full'); addr.setAttribute('aria-label', 'Runner address');
    var arow = addr;
    var to = el('input'); to.placeholder = 'to: ckb1…/ckt1… address'; to.size = 50;
    var amt = el('input'); amt.placeholder = 'CKB'; amt.size = 10;
    var send = el('button', 'btn', 'Send'); send.type = 'button';
    send.addEventListener('click', function () {
      if (!to.value.trim() || !amt.value.trim()) return say('Enter the address and the amount.', true);
      api('/api/wallet/send', { method: 'POST', body: { to: to.value.trim(), ckb: amt.value.trim() } })
        .then(function (j) { say('Sent: ' + j.tx_hash); load(); })
        .catch(function (x) { say(x.message, true); });
    });
    var srow = el('div', 'row'); [to, amt, send].forEach(function (n) { srow.appendChild(n); });
    [el('h3', '', 'Runner address'), arow, info, el('h3', '', 'Send'), srow].forEach(function (n) { c.appendChild(n); });

    var cr = card('Stake and registration');
    cr.appendChild(el('p', '', 'The stake is a Nervos DAO deposit of at least 10,000 CKB in your own wallet; it stays there. Its owner consents by signing a one-line text.'));
    // Auto renew (registry.auto_renew, on by default): its own setting, independent of the service runtime.
    var autoBox = el('div', 'setting');
    var autoTxt = el('div');
    autoTxt.appendChild(el('p', 't', 'Auto renew'));
    var autoDesc = el('p', 'd', 'Renews the attestation 2 days before it expires.');
    autoTxt.appendChild(autoDesc);
    var sw = el('button', 'switch'); sw.type = 'button'; sw.setAttribute('role', 'switch'); sw.setAttribute('aria-label', 'Auto renew');
    sw.appendChild(el('span'));
    autoBox.appendChild(autoTxt); autoBox.appendChild(sw);
    cr.appendChild(autoBox);
    var tlBox = el('div', 'tl');
    cr.appendChild(tlBox);
    sw.addEventListener('click', function () {
      var on = sw.getAttribute('aria-checked') !== 'true';
      sw.disabled = true;
      api('/api/settings', { method: 'PUT', body: { registry: { auto_renew: on } } })
        .then(function () { say(on ? 'Auto renew is on.' : 'Auto renew is off: renew by hand before the attestation expires.'); load(); })
        .catch(function (x) { say(x.message, true); })
        .then(function () { sw.disabled = false; });
    });
    var rinfo = el('div');
    var stake = el('input'); stake.placeholder = 'stake: DAO deposit 0x<tx hash>:<index>'; stake.size = 50;
    var getText = el('button', 'btn', 'Show text to sign'); getText.type = 'button';
    var textBox = el('div'); textBox.hidden = true;
    var text = el('textarea'); text.readOnly = true; text.rows = 4; text.cols = 70; text.setAttribute('aria-label', 'Text to sign');
    var owner = el('p');
    var trow = el('div', 'row'); trow.appendChild(text); trow.appendChild(copyBtn(function () { return text.value; }, text));
    [el('label', '', 'Text to sign'), trow, owner,
      el('p', '', 'Sign this text in your wallet with the address of the DAO deposit (Neuron: Tools → Sign/Verify Message) and paste the signature below.')]
      .forEach(function (n) { textBox.appendChild(n); });
    var sig = el('input'); sig.placeholder = 'Signature 0x…'; sig.size = 50; sig.setAttribute('aria-label', 'Signature');
    var reg = el('button', 'btn', 'Register'); reg.type = 'button';
    var rm = el('button', 'btn danger', 'Remove registration'); rm.type = 'button';
    var renew = el('button', 'btn', 'Renew now'); renew.type = 'button';
    var ask = el('div', 'confirm'); ask.hidden = true; ask.setAttribute('role', 'alert');
    var yes = el('button', 'btn danger', 'Yes, remove'); yes.type = 'button';
    var no = el('button', 'btn', 'Cancel'); no.type = 'button';
    [el('span', '', 'Remove the registration? The node leaves the ranking; its CKB returns to the runner wallet.'), yes, no].forEach(function (n) { ask.appendChild(n); });
    getText.addEventListener('click', function () {
      if (!stake.value.trim()) return say('Enter the stake (out point of your Nervos DAO deposit).', true);
      api('/api/registry/consent-message', { method: 'POST', body: { stake: stake.value.trim() } }).then(function (j) {
        text.value = j.message;
        owner.textContent = 'Stake owner: ' + j.stake_owner_address;
        textBox.hidden = false;
        say('');
      }).catch(function (x) { say(x.message, true); });
    });
    reg.addEventListener('click', function () {
      if (!stake.value.trim() || !sig.value.trim()) return say('Enter the stake and the signature.', true);
      api('/api/registry/create', { method: 'POST', body: { stake: stake.value.trim(), consent: sig.value.trim() } })
        .then(function (j) { say((j.updated ? 'Registration updated: ' : 'Registered: ') + j.tx_hash); load(); })
        .catch(function (x) { say(x.message, true); });
    });
    renew.addEventListener('click', function () {
      api('/api/registry/refresh', { method: 'POST', body: {} })
        .then(function (j) { say('Attestation renewed: ' + j.tx_hash); load(); })
        .catch(function (x) { say(x.message, true); });
    });
    rm.addEventListener('click', function () { ask.hidden = false; });
    no.addEventListener('click', function () { ask.hidden = true; });
    yes.addEventListener('click', function () {
      ask.hidden = true;
      api('/api/registry/remove', { method: 'POST', body: {} })
        .then(function (j) { say('Removed: ' + j.tx_hash); load(); })
        .catch(function (x) { say(x.message, true); });
    });
    var r1 = el('div', 'row'); [stake, getText].forEach(function (n) { r1.appendChild(n); });
    var r2 = el('div', 'row'); [sig, reg, renew, rm].forEach(function (n) { r2.appendChild(n); });
    [rinfo, r1, textBox, r2, ask].forEach(function (n) { cr.appendChild(n); });
    var lockBox = el('div');
    root.appendChild(lockBox); root.appendChild(c); root.appendChild(cr); root.appendChild(msg);
    function load() {
      var none = { registered: false, stake: null, unavailable: true };
      var werr = null;
      Promise.all([api('/api/wallet').catch(function (x) { werr = x.message; return {}; }), api('/api/registry').catch(function (x) { none.error = x.message; return none; }), api('/api/runner-cell').catch(function () { return {}; })]).then(function (r) {
        var w = r[0], ls = w.last_sweep;
        if (r[1].locked) {
          // Mainnet before 1.x: no registration; the wallet stays usable to send back CKB sent by mistake.
          lockBox.textContent = '';
          lockBox.appendChild(lockedNotice(r[1].message));
          intro.textContent = 'Nothing needs to be sent here on mainnet now. CKB that arrived here can be sent back with "Send" below.';
          cr.hidden = true;
        } else {
          lockBox.textContent = '';
          cr.hidden = false;
        }
        warn.textContent = w.sweep_warning || '';
        warn.style.display = w.sweep_warning ? '' : 'none';
        addr.textContent = r[2].runner_address || w.address || '';
        info.textContent = '';
        if (werr) { info.appendChild(el('p', 'msg err', 'Wallet unavailable: ' + werr)); }
        else dl(info, [
          ['Balance', ckb(w.capacity_shannons)],
          ['Sweep to', w.sweep_address || 'not set (payout.sweep_address)'],
          ['Sweep', w.sweep_amount_ckb + ' CKB when the wallet reaches ' + w.sweep_threshold_ckb + ' CKB'],
          ['Last sweep', ls ? ckb(ls.amount_shannons) + ' – ' + new Date(ls.at_ms).toLocaleString() : 'never'],
        ]);
        if (r[1].locked) return;
        reg.textContent = r[1].registered ? 'Update stake' : 'Register';
        rinfo.textContent = '';
        var st = r[1].stake;
        if (r[1].unavailable) {
          var mainnet = /not deployed on mainnet/.test(r[1].error || '');
          rinfo.appendChild(el('p', 'msg', mainnet ? 'The CSR scripts are not deployed on mainnet yet.' : (r[1].error || 'Registry unavailable.')));
          return;
        }
        var auto = r[1].auto_renew !== false;
        sw.setAttribute('aria-checked', auto ? 'true' : 'false');
        autoDesc.textContent = auto
          ? 'The runner renews the attestation 2 days before it expires, also while the service runtime is off. Each renewal costs one small transaction fee from this wallet.'
          : 'Off: the attestation expires and the node leaves the ranking unless you click "Renew now" in time. The Overview warns 3 days before.';
        tlBox.textContent = '';
        function step(done, cls, title, sub) {
          var d = el('div', 'tl-step'), i = el('i', cls), t = el('div');
          if (done) i.className = 'done';
          t.appendChild(el('p', '', title)); t.appendChild(el('p', '', sub));
          d.appendChild(i); d.appendChild(t); tlBox.appendChild(d);
        }
        var exp = r[1].attestation_expiry_ms;
        if (r[1].registered) {
          step(true, '', 'Registered', r[1].outpoint);
          step(true, '', 'Attestation from the monitor', 'valid until ' + fmtDate(exp) + ' · ' + fmtLeft(exp) + ' left');
          if (auto) step(false, '', 'Next renewal', fmtDate(r[1].next_renewal_ms) + ', automatic');
          else step(false, 'warn', 'Attestation expires', fmtDate(exp) + ': renew by hand before then');
        } else {
          step(false, 'warn', 'Not registered', 'enter the stake below, sign the text and register');
        }
        dl(rinfo, [
          ['Registered', r[1].registered ? r[1].outpoint : 'no'],
          ['Attestation expires', exp ? new Date(exp).toLocaleString() : '–'],
          ['Last renewal error', r[1].renewal_error || '–'],
          ['Stake', st ? st.outpoint : '–'],
          ['Stake status', st ? st.status : '–'],
          ['Stake amount', st ? ckb(st.capacity_shannons) : '–'],
        ]);
      }).catch(function (x) { info.textContent = x.message; });
    }
    load();
  };

  var JOB_TABS = [
    ['Pending', ['Discovered', 'Queued', 'Scheduled']],
    ['Active', ['Running', 'Paused', 'Completed', 'Submitting']],
    ['Closed', ['Settled']],
    ['Failed', ['Failed', 'Rejected', 'Expired', 'Lost']],
  ];
  function ckb(sh) { return typeof sh === 'number' ? (sh / 1e8).toFixed(8).replace(/\.?0+$/, '') + ' CKB' : '–'; }
  function short(h) { return h && h.length > 20 ? h.slice(0, 10) + '…' + h.slice(-6) : (h || '–'); }

  pages.jobs = function (root) {
    var c = card('Jobs');
    var tabs = el('div', 'row');
    var table = el('div');
    var detail = el('div');
    var cur = 0;
    var open = null;
    JOB_TABS.forEach(function (t, i) {
      var b = el('button', 'btn', t[0]); b.type = 'button';
      b.addEventListener('click', function () { cur = i; load(); });
      tabs.appendChild(b);
    });
    c.appendChild(tabs); c.appendChild(table);
    root.appendChild(c);
    lockedTop(root);
    root.appendChild(detail);
    function showDetail(id) {
      open = id;
      api('/api/jobs/' + id).then(function (j) {
        detail.textContent = '';
        var d = card('Job ' + j.id);
        dl(d, [
          ['Outpoint', j.outpoint], ['Service', j.code_hash], ['Kind', j.kind], ['Trigger', j.trigger],
          ['Tip', ckb(j.tip_shannons)], ['Tip rate (CKB/M cycles)', j.tip_rate], ['Cycles used', j.cycles == null ? '–' : j.cycles],
          ['State', j.state], ['Result tx', j.result_tx || '–'], ['Result', j.result_hex || '–'], ['Error', j.error || '–'],
          ['Limits', 'cycles ' + j.limits.max_cycles + ', memory ' + j.limits.max_memory_mb + ' MiB, wall ' + j.limits.max_wall_seconds + ' s'],
        ]);
        d.appendChild(el('h2', '', 'Timeline'));
        var tl = el('table');
        j.timeline.forEach(function (e) {
          var r = el('tr');
          [fmtTime(e.t), e.state, e.note || ''].forEach(function (x) { r.appendChild(el('td', '', x)); });
          tl.appendChild(r);
        });
        d.appendChild(tl);
        d.appendChild(el('h2', '', 'Logs'));
        d.appendChild(el('pre', 'log', (j.logs || []).join('\n') || '(none)'));
        detail.appendChild(d);
      }).catch(function (x) { detail.textContent = x.message; });
    }
    function load() {
      Array.prototype.forEach.call(tabs.children, function (b, i) { b.className = 'btn' + (i === cur ? ' primary' : ''); });
      api('/api/jobs').then(function (j) {
        table.textContent = '';
        var states = JOB_TABS[cur][1];
        var list = j.jobs.filter(function (x) { return states.indexOf(x.state) >= 0; });
        if (!list.length) { table.appendChild(el('p', 'msg', 'No jobs.')); return; }
        var t = el('table'), h = el('tr');
        ['ID', 'Service', 'Kind', 'Trigger', 'Tip', 'Tip rate', 'Cycles', 'Result tx', 'Status'].forEach(function (x) { h.appendChild(el('th', '', x)); });
        t.appendChild(h);
        list.forEach(function (x) {
          var r = el('tr', 'click');
          [String(x.id), short(x.code_hash), x.kind, x.trigger, ckb(x.tip_shannons), String(x.tip_rate),
            x.cycles == null ? '–' : String(x.cycles), short(x.result_tx)].forEach(function (v) { r.appendChild(el('td', '', v)); });
          var st = el('td'); st.appendChild(pill(x.state)); r.appendChild(st);
          r.addEventListener('click', function () { showDetail(x.id); });
          t.appendChild(r);
        });
        table.appendChild(t);
      }).catch(function (x) { table.textContent = x.message; });
      if (open != null) showDetail(open);
    }
    load();
    timers.push(setInterval(load, 5000));
  };

  pages.earnings = function (root) {
    var c = card('Earnings');
    var info = el('div'), chartBox = el('div'), table = el('div');
    var btns = el('div', 'row');
    var pa = el('a', 'btn', 'Wallet & registration'); pa.href = '#/wallet';
    btns.appendChild(pa);
    var warn = el('p', 'msg err'); warn.style.display = 'none';
    [warn, info, btns, el('h2', '', 'Tips per day'), chartBox, el('h2', '', 'Settlements'), table].forEach(function (n) { c.appendChild(n); });
    root.appendChild(c);
    lockedTop(root);
    function load() {
      api('/api/wallet').then(function (w) {
        warn.textContent = w.sweep_warning || '';
        warn.style.display = w.sweep_warning ? '' : 'none';
      }).catch(function () {});
      api('/api/earnings').then(function (e) {
        info.textContent = '';
        dl(info, [['Runner wallet', ckb((e.wallet || {}).capacity_shannons)], ['Tips earned (verified on chain)', ckb(e.tips_shannons)], ['Fees paid', ckb(e.fees_shannons)]]);
        var days = {};
        e.settlements.forEach(function (x) {
          var d = new Date((x.t || 0) * 1000).toISOString().slice(0, 10);
          days[d] = (days[d] || 0) + x.tip_shannons;
        });
        var keys = Object.keys(days).sort();
        chartBox.textContent = '';
        if (!keys.length) chartBox.appendChild(el('p', 'msg', 'No settlements yet.'));
        else {
          var max = Math.max.apply(null, keys.map(function (k) { return days[k]; })), w = 40, H = 120;
          var svg = document.createElementNS(NS, 'svg');
          svg.setAttribute('viewBox', '0 0 ' + (keys.length * w + 10) + ' ' + (H + 30));
          svg.setAttribute('class', 'chart');
          keys.forEach(function (k, i) {
            var h = Math.max(2, Math.round(days[k] / max * H));
            var r = document.createElementNS(NS, 'rect');
            r.setAttribute('x', 5 + i * w); r.setAttribute('y', H - h); r.setAttribute('width', w - 8); r.setAttribute('height', h);
            r.setAttribute('fill', 'currentColor');
            var t = document.createElementNS(NS, 'title'); t.textContent = k + ': ' + ckb(days[k]);
            r.appendChild(t); svg.appendChild(r);
            var l = document.createElementNS(NS, 'text');
            l.setAttribute('x', 5 + i * w); l.setAttribute('y', H + 14); l.setAttribute('font-size', '8'); l.textContent = k.slice(5);
            svg.appendChild(l);
          });
          chartBox.appendChild(svg);
        }
        table.textContent = '';
        if (!e.settlements.length) { table.appendChild(el('p', 'msg', 'None.')); return; }
        var t = el('table'), h = el('tr');
        ['Job', 'Transaction', 'Tip', 'Block'].forEach(function (x) { h.appendChild(el('th', '', x)); });
        t.appendChild(h);
        e.settlements.forEach(function (x) {
          var r = el('tr');
          [short(x.job_outpoint), short(x.tx_hash), ckb(x.tip_shannons), String(x.block)].forEach(function (v) { r.appendChild(el('td', '', v)); });
          t.appendChild(r);
        });
        table.appendChild(t);
      }).catch(function (x) { info.textContent = x.message; });
    }
    load();
    timers.push(setInterval(load, 5000));
  };

  pages.logs = function (root) {
    var c = card('Logs');
    var row = el('div', 'row'), sel = el('select');
    ['error', 'warn', 'info', 'debug', 'trace'].forEach(function (l) { var o = el('option', '', l); o.value = l; sel.appendChild(o); });
    sel.value = 'info';
    var dlb = el('button', 'btn', 'Download'); dlb.type = 'button';
    var pre = el('pre', 'log');
    row.appendChild(sel); row.appendChild(dlb);
    c.appendChild(row); c.appendChild(pre);
    root.appendChild(c);
    var text = '';
    function load() {
      api('/api/logs?level=' + sel.value).then(function (j) {
        text = j.lines.join('\n');
        pre.textContent = text;
        pre.scrollTop = pre.scrollHeight;
      }).catch(function (x) { pre.textContent = x.message; });
    }
    sel.addEventListener('change', load);
    dlb.addEventListener('click', function () {
      var u = URL.createObjectURL(new Blob([text + '\n'], { type: 'text/plain' }));
      var a = el('a'); a.href = u; a.download = 'csr.log';
      document.body.appendChild(a); a.click(); a.remove();
      setTimeout(function () { URL.revokeObjectURL(u); }, 1000);
    });
    load();
    timers.push(setInterval(load, 10000));
  };

  var onStatus = null;

  function updateBadge() {
    var b = document.getElementById('network-badge');
    var s = state.status;
    var n = s && s.network;
    if (!n) return;
    b.textContent = n.toUpperCase();
    b.className = 'badge ' + n;
    var peer = document.getElementById('peer-dashboard');
    if (peer) {
      if (s.peer_dashboard) {
        peer.href = s.peer_dashboard;
        peer.textContent = n === 'mainnet' ? 'Testnet dashboard' : 'Mainnet dashboard';
        peer.hidden = false;
      } else {
        peer.hidden = true;
      }
    }
    var t = document.getElementById('host-title'), l = document.getElementById('host-line'), f = document.getElementById('foot-version');
    if (t) t.textContent = (s.hostname ? s.hostname + ' · ' : '') + 'Service Runner';
    if (l) l.textContent = 'CKB Service Runner ' + s.version + ' · ' + n + (typeof s.uptime_s === 'number' ? ' · up ' + fmtUp(s.uptime_s) : '');
    if (f) f.textContent = 'CKB Service Runner ' + s.version + ' · ' + n;
    var src = document.getElementById('foot-source');
    if (src && s.links && s.links.source) { src.href = s.links.source; src.textContent = s.links.source.replace(/^https:\/\//, ''); }
  }
  function setStatus(s) {
    state.status = s;
    updateBadge();
    if (onStatus) onStatus();
  }

  function route() {
    timers.forEach(clearInterval); timers = []; onStatus = null;
    var name = (location.hash.replace(/^#\//, '') || 'overview');
    if (!pages[name]) name = 'overview';
    document.querySelectorAll('#nav a').forEach(function (a) {
      a.className = a.getAttribute('href') === '#/' + name ? 'active' : '';
    });
    if (!state.auth) return renderLogin();
    view.textContent = '';
    pages[name](view);
    if (!state.pwReq && (name === 'wallet' || name === 'settings')) {
      pwNotice('No dashboard password is set: sending CKB, registration, service actions and settings changes are refused until you set one with "csr set-password".');
    }
  }

  function poll() {
    api('/api/status').then(setStatus).catch(function () {});
  }
  function connect() {
    if (es) es.close();
    if (!window.EventSource) return;
    es = new EventSource('/api/events');
    es.addEventListener('status', function (e) {
      try { setStatus(JSON.parse(e.data)); } catch (x) { /* ignore */ }
    });
  }

  function theme(t) {
    document.documentElement.setAttribute('data-theme', t);
    var tb = document.getElementById('theme-toggle');
    if (tb) tb.textContent = t === 'dark' ? 'Light theme' : 'Dark theme';
    try { localStorage.setItem('csr-theme', t); } catch (x) { /* ignore */ }
  }
  var saved = null;
  try { saved = localStorage.getItem('csr-theme'); } catch (x) { /* ignore */ }
  theme(saved || (window.matchMedia && matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'));
  document.getElementById('theme-toggle').addEventListener('click', function () {
    theme(document.documentElement.getAttribute('data-theme') === 'dark' ? 'light' : 'dark');
  });

  window.addEventListener('hashchange', route);
  fetch('/api/session', { credentials: 'same-origin' }).then(function (r) { return r.json(); }).then(function (s) {
    state.csrf = s.csrf_token; state.auth = s.authenticated; state.pwReq = s.password_required;
    route();
    if (state.auth) { poll(); connect(); }
  }).catch(function () { view.textContent = 'Cannot reach CSR.'; });
  setInterval(function () { if (state.auth && (!es || es.readyState !== 1)) poll(); }, 10000);
})();
