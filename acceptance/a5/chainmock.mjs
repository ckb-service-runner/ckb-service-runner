// A CKB node mock with an indexer: get_cells answers from a list of cells and records every search key.
// All other methods are forwarded to the mock node of acceptance/a1/lib.mjs.
import http from 'node:http';
import { startMockNode, freePort, onCleanup } from '../a1/lib.mjs';

export async function startChainMock(initial = {}) {
  const node = await startMockNode(initial);
  const searches = [];
  const cells = [];
  async function call(req) {
    if (req && req.method === 'get_cells') {
      const key = req.params[0];
      searches.push(key);
      const same = (a, b) => a && b && a.code_hash === b.code_hash && a.hash_type === b.hash_type && a.args === b.args;
      const objects = cells.filter((c) => (key.script_type === 'type' ? same(c.output.type, key.script) : same(c.output.lock, key.script)));
      return { jsonrpc: '2.0', id: req.id, result: { objects, last_cursor: null } };
    }
    const r = await fetch(node.url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(req) });
    return r.json();
  }
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', async () => {
      let parsed;
      try { parsed = JSON.parse(body); } catch { res.statusCode = 400; res.end(); return; }
      const out = Array.isArray(parsed) ? await Promise.all(parsed.map(call)) : await call(parsed);
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify(out));
    });
  });
  const port = await freePort();
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  const close = () => new Promise((r) => { server.closeAllConnections(); server.close(() => r()); });
  onCleanup(close);
  return { port, url: `http://127.0.0.1:${port}`, node, searches, cells, close };
}
