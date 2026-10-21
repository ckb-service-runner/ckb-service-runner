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
        if (!r.ok) throw new Error(j.error || 'HTTP ' + r.status);
        return j;
      });
    });
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
  function pct(v) { return typeof v === 'number' ? (v * 100).toFixed(2) + '%' : '–'; }
  function clear(c) { while (c.children.length > 1) c.removeChild(c.lastChild); }
  function stale(c, j) {
    if (j.stale) c.appendChild(el('p', 'msg stale', 'Stale data: showing the last cached monitor reply. ' + (j.error || '')));
    else if (j.error) c.appendChild(el('p', 'msg err', j.error));
  }
  function checkType(t) { return t === 'p2p_verified' ? 'P2P-verified' : t === 'challenge_only' ? 'challenge-only' : t; }
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

  pages.overview = function (root) {
    var g = el('div', 'grid');
    var cs = card('Network'), cn = card('Node'), cr = card('Runtime'), cres = card('Resources');
    var cf = card('Wallet'), ck = card('CSR monitor');
    [cs, cn, cr, cres, cf, ck].forEach(function (c) { g.appendChild(c); });
    root.appendChild(g);
    function paint() {
      var s = state.status;
      if (!s) return;
      [cs, cn, cr, ck].forEach(function (c) { while (c.children.length > 1) c.removeChild(c.lastChild); });
      dl(cs, [['Network', s.network], ['Version', s.version], ['Uptime', fmtUp(s.uptime_s)], ['Peer ID', s.peer_id], ['Service key', s.service_pubkey]]);
      dl(cn, [['R2 state', pill(s.r2.state)], ['Reasons', (s.r2.reasons || []).join('; ') || 'none']]);
      dl(cr, [['State', pill(s.runtime.state)], ['Reason', s.runtime.reason], ['Mainnet unlock', s.mainnet_runtime_unlocked ? 'yes' : 'no']]);
      var locked = s.network === 'mainnet' && !s.mainnet_runtime_unlocked;
      var tg = el('label', 'runtime-toggle' + (locked ? ' disabled' : ''));
      var cb = document.createElement('input');
      cb.type = 'checkbox'; cb.id = 'runtime-toggle'; cb.checked = s.runtime.state === 'active'; cb.disabled = locked;
      tg.appendChild(cb); tg.appendChild(document.createTextNode(' Runtime'));
      cr.appendChild(tg);
      if (locked) cr.appendChild(el('p', 'msg', 'Services start on mainnet in Phase 2'));
      var r = s.ranking || {};
      dl(ck, [['Join', r.join ? 'yes' : 'no'], ['Mode', r.ping_mode], ['Last check ok', r.last_check_ok_at ? new Date(r.last_check_ok_at * 1000).toLocaleString() : 'never']]);
      if (r.join) {
        var rk = el('div');
        ck.appendChild(rk);
        api('/api/network').then(function (j) {
          var n = j.node;
          dl(rk, [['Rank', n ? n.rank + ' / ' + n.of : 'not ranked'], ['Availability 30 d', n ? pct(n.availability.d30) : '–']]);
          if (j.stale) rk.appendChild(el('p', 'msg stale', 'stale'));
          var ol = el('a', '', 'Open CSR monitor'); ol.href = '#/network'; rk.appendChild(ol);
        }).catch(function () {});
      } else {
        ck.appendChild(el('p', '', 'Join the CSR monitor to see stats'));
      }
    }
    function payoutCard() {
      api('/api/earnings').then(function (e) {
        while (cf.children.length > 1) cf.removeChild(cf.lastChild);
        dl(cf, [['Balance', ckb((e.wallet || {}).capacity_shannons)], ['Tips earned', ckb(e.tips_shannons)], ['Settlements', e.settlements.length]]);
        var a = el('a', '', 'Wallet & registration'); a.href = '#/wallet'; cf.appendChild(a);
      }).catch(function () {});
    }
    function res() {
      api('/api/resources').then(function (r) {
        while (cres.children.length > 1) cres.removeChild(cres.lastChild);
        dl(cres, [
          ['CPU (total / csr / ckb)', r.cpu.total_percent + '% / ' + r.cpu.csr_percent + '% / ' + (r.cpu.ckb_percent === null ? '–' : r.cpu.ckb_percent + '%')],
          ['RAM free', r.ram.free_mb + ' / ' + r.ram.total_mb + ' MB'],
          ['Disk free', r.disk.free_gb + ' / ' + r.disk.total_gb + ' GB']]);
      }).catch(function () {});
    }
    onStatus = paint; paint(); res(); payoutCard();
    timers.push(setInterval(payoutCard, 10000));
    timers.push(setInterval(res, 5000));
  };

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
    function drawMap(nw) {
      var ok = !!(nw && nw.locations_available);
      var list = ok ? (nw.locations || []) : [];
      mapBox.innerHTML = window.CSRWorldMap.svg(list, {label: 'CSR runners on a world map'});
      dots = window.CSRWorldMap.dots(list);
      mapInfo.textContent = ok ? 'Tap a dot for details.' : 'Location data is not available from the monitor.';
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
        drawMap(nw);
        dl(cw, [['Nodes worldwide', nw ? nw.nodes_total : null], ['Nodes reachable', nw ? nw.nodes_reachable : null],
          ['Updated', nw ? fmtTime(nw.updated_at) : null], ['Fetched', fmtTime(j.fetched_at)], ['Monitor', j.monitor_url]]);
        var ping = j.ping || {};
        if (n) {
          var a = n.attestation || {};
          dl(cr, [['Rank', n.rank + ' / ' + n.of], ['Availability 7 d', pct(n.availability.d7)], ['Availability 30 d', pct(n.availability.d30)],
            ['Availability 365 d', pct(n.availability.d365)], ['Check type', checkType(n.check_type)],
            ['Attestation', a.status + (a.expiry ? ' (expires ' + fmtTime(a.expiry) + ')' : '')],
            ['Stake', n.stake ? n.stake.status + (n.stake.capacity_ckb != null ? ' (' + n.stake.capacity_ckb + ' CKB)' : '') : null],
            ['Last check', fmtTime(n.last_check_at)], ['Ping mode', ping.mode], ['Last successful check', fmtTime(ping.last_check_ok_at)]]);
        } else {
          cr.appendChild(el('p', '', nw ? 'Not ranked yet.' : 'No data available.'));
          dl(cr, [['Ping mode', ping.mode], ['Last successful check', fmtTime(ping.last_check_ok_at)]]);
        }
        if (nw && nw.countries && nw.countries.length) cc.appendChild(countryBars(nw.countries));
        else cc.appendChild(el('p', '', 'No country data.'));
      }).catch(function (x) { clear(cw); cw.appendChild(el('p', 'msg err', x.message)); });
    }
    load();
    timers.push(setInterval(load, 30000));
  };

  pages.node = function (root) {
    var c = card('Node'), ct = card('Tip lag (24 h)'), cp = card('Tx pool & peers');
    root.appendChild(c); root.appendChild(ct); root.appendChild(cp);
    function load() {
      api('/api/node').then(function (n) {
        while (c.children.length > 1) c.removeChild(c.lastChild);
        while (cp.children.length > 1) cp.removeChild(cp.lastChild);
        dl(c, [['Chain', n.chain], ['Version', n.node_version], ['Type', n.node_type], ['Tip', n.tip_number], ['Tip hash', n.tip_hash],
          ['Best known', n.best_known], ['Tip lag', n.tip_lag], ['Epoch', n.epoch], ['Indexer tip', n.indexer_tip], ['Indexer lag', n.indexer_lag]]);
        dl(cp, [['Pending', n.tx_pool && n.tx_pool.pending], ['Proposed', n.tx_pool && n.tx_pool.proposed], ['Peers in', n.peers_in], ['Peers out', n.peers_out]]);
      }).catch(function () {});
      api('/api/resources/history').then(function (h) {
        while (ct.children.length > 1) ct.removeChild(ct.lastChild);
        ct.appendChild(chart(h.samples, 'tip_lag', 'blocks behind'));
      }).catch(function () {});
    }
    load();
    timers.push(setInterval(load, 10000));
  };

  pages.resources = function (root) {
    var c = card('Now'), c1 = card('CPU % (24 h)'), c2 = card('Free RAM MB (24 h)');
    root.appendChild(c); root.appendChild(c1); root.appendChild(c2);
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
      }).catch(function () {});
    }
    load();
    timers.push(setInterval(load, 10000));
  };

  var FIELDS = [
    ['network', 'Network', 'select:testnet,mainnet', true],
    ['node.rpc_url', 'Node RPC URL', 'text', true],
    ['node.node_type', 'Node type', 'select:auto,ckb,neuron'],
    ['dashboard.bind', 'Dashboard bind', 'text', true],
    ['ranking.join', 'Join the CSR monitor', 'bool'],
    ['ranking.ping_mode', 'Ping mode', 'select:inbound,outbound,both'],
    ['ranking.ping_bind', 'Ping bind', 'text', true],
    ['monitor.url', 'Monitor URL', 'text'],
    ['monitor.pubkey', 'Monitor pubkey', 'text'],
    ['payout.sweep_address', 'Sweep to address', 'text'],
    ['payout.sweep_threshold_ckb', 'Sweep when the wallet reaches (CKB)', 'int'],
    ['payout.sweep_amount_ckb', 'Sweep amount (CKB)', 'int'],
    ['runtime.enabled', 'Runtime enabled', 'bool'],
    ['runtime.min_ckb_per_mcycle', 'Min CKB per Mcycle', 'number'],
    ['limits.max_concurrent_jobs', 'Max concurrent jobs', 'int'],
    ['limits.max_cycles_per_job', 'Max cycles per job', 'int'],
    ['limits.max_vm_memory_mb', 'Max VM memory (MB)', 'int'],
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
    api('/api/settings').then(function (cfg) {
      var f = el('form'), inputs = [];
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
    function copyBtn(get) {
      var b = el('button', 'btn', 'Copy'); b.type = 'button';
      b.addEventListener('click', function () {
        var v = get();
        if (navigator.clipboard && v) navigator.clipboard.writeText(v).then(function () { say('Copied.'); }, function () { say('Copy failed; select the text and copy it by hand.', true); });
      });
      return b;
    }
    var c = card('Runner wallet');
    c.appendChild(el('p', '', 'Send about 500 CKB here from your wallet to start. The registration uses about 161 CKB, the rest pays small fees. Tips are added here.'));
    var info = el('div');
    var addr = el('input'); addr.readOnly = true; addr.size = 70; addr.setAttribute('aria-label', 'Runner address');
    var arow = el('div', 'row'); arow.appendChild(addr); arow.appendChild(copyBtn(function () { return addr.value; }));
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
    var rinfo = el('div');
    var stake = el('input'); stake.placeholder = 'stake: DAO deposit 0x<tx hash>:<index>'; stake.size = 50;
    var getText = el('button', 'btn', 'Show text to sign'); getText.type = 'button';
    var textBox = el('div'); textBox.hidden = true;
    var text = el('textarea'); text.readOnly = true; text.rows = 4; text.cols = 70; text.setAttribute('aria-label', 'Text to sign');
    var owner = el('p');
    var trow = el('div', 'row'); trow.appendChild(text); trow.appendChild(copyBtn(function () { return text.value; }));
    [el('label', '', 'Text to sign'), trow, owner,
      el('p', '', 'Sign this text in your wallet with the address of the DAO deposit (Neuron: Tools → Sign/Verify Message) and paste the signature below.')]
      .forEach(function (n) { textBox.appendChild(n); });
    var sig = el('input'); sig.placeholder = 'Signature 0x…'; sig.size = 50; sig.setAttribute('aria-label', 'Signature');
    var reg = el('button', 'btn', 'Register'); reg.type = 'button';
    var rm = el('button', 'btn', 'Remove registration'); rm.type = 'button';
    var ask = el('div', 'row'); ask.hidden = true;
    var yes = el('button', 'btn', 'Yes, remove'); yes.type = 'button';
    var no = el('button', 'btn', 'Cancel'); no.type = 'button';
    [el('span', '', 'Remove the registration? Its CKB returns to the runner wallet.'), yes, no].forEach(function (n) { ask.appendChild(n); });
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
    rm.addEventListener('click', function () { ask.hidden = false; });
    no.addEventListener('click', function () { ask.hidden = true; });
    yes.addEventListener('click', function () {
      ask.hidden = true;
      api('/api/registry/remove', { method: 'POST', body: {} })
        .then(function (j) { say('Removed: ' + j.tx_hash); load(); })
        .catch(function (x) { say(x.message, true); });
    });
    var r1 = el('div', 'row'); [stake, getText].forEach(function (n) { r1.appendChild(n); });
    var r2 = el('div', 'row'); [sig, reg, rm].forEach(function (n) { r2.appendChild(n); });
    [rinfo, r1, textBox, r2, ask].forEach(function (n) { cr.appendChild(n); });
    root.appendChild(c); root.appendChild(cr); root.appendChild(msg);
    function load() {
      Promise.all([api('/api/wallet'), api('/api/registry'), api('/api/runner-cell')]).then(function (r) {
        var w = r[0], ls = w.last_sweep;
        addr.value = r[2].runner_address || w.address || '';
        info.textContent = '';
        dl(info, [
          ['Balance', ckb(w.capacity_shannons)],
          ['Sweep to', w.sweep_address || 'not set (payout.sweep_address)'],
          ['Sweep', w.sweep_amount_ckb + ' CKB when the wallet reaches ' + w.sweep_threshold_ckb + ' CKB'],
          ['Last sweep', ls ? ckb(ls.amount_shannons) + ' – ' + new Date(ls.at_ms).toLocaleString() : 'never'],
        ]);
        reg.textContent = r[1].registered ? 'Update stake' : 'Register';
        rinfo.textContent = '';
        var st = r[1].stake;
        dl(rinfo, [
          ['Registered', r[1].registered ? r[1].outpoint : 'no'],
          ['Attestation expiry (ms)', r[1].attestation_expiry_ms],
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
    [info, btns, el('h2', '', 'Tips per day'), chartBox, el('h2', '', 'Settlements'), table].forEach(function (n) { c.appendChild(n); });
    root.appendChild(c);
    function load() {
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
    var n = state.status && state.status.network;
    if (!n) return;
    b.textContent = n.toUpperCase();
    b.className = 'badge ' + n;
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
