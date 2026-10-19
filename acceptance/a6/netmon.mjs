// Mock of the CSR Network Monitor public API, with optional GET /v1/locations.
import http from 'node:http';
import { freePort, onCleanup } from '../a1/lib.mjs';

export const NETWORK = {
  nodes_total: 42, nodes_reachable: 30,
  countries: [{ code: 'DE', count: 12 }, { code: 'US', count: 9 }],
  updated_at: 1792000000,
};

// locations: undefined -> 404, 'bad' -> 200 with a body that is not JSON, object -> served as JSON.
export async function startMapMonitor({ locations, down = false } = {}) {
  const hits = {};
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://x');
    const key = url.pathname.startsWith('/v1/nodes/') ? '/v1/nodes' : url.pathname;
    hits[key] = (hits[key] || 0) + 1;
    res.setHeader('content-type', 'application/json');
    if (down) { res.statusCode = 503; res.end('{"error":"down"}'); return; }
    if (req.method === 'GET' && url.pathname === '/v1/network') { res.end(JSON.stringify(NETWORK)); return; }
    if (req.method === 'GET' && url.pathname === '/v1/locations' && locations !== undefined) {
      res.end(locations === 'bad' ? 'not json' : JSON.stringify(locations));
      return;
    }
    res.statusCode = 404;
    res.end('{"error":"not_found"}');
  });
  const port = await freePort();
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  const close = () => new Promise((r) => { server.closeAllConnections(); server.close(() => r()); });
  onCleanup(close);
  return { url: `http://127.0.0.1:${port}`, hits, close };
}
