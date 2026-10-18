// Mock of the CSR Network Monitor public API.
import http from 'node:http';
import { freePort, onCleanup } from '../a1/lib.mjs';

export const NETWORK = {
  nodes_total: 42, nodes_reachable: 30,
  countries: [{ code: 'DE', count: 12 }, { code: 'US', count: 9 }, { code: 'JP', count: 5 }],
  updated_at: 1792000000,
};

export async function startPublicMonitor({ nodes = {}, down = false } = {}) {
  const hits = {};
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://x');
    const key = url.pathname.startsWith('/v1/nodes/') ? '/v1/nodes' : url.pathname;
    hits[key] = (hits[key] || 0) + 1;
    res.setHeader('content-type', 'application/json');
    if (down) { res.statusCode = 503; res.end('{"error":"down"}'); return; }
    if (req.method === 'GET' && url.pathname === '/v1/network') { res.end(JSON.stringify(NETWORK)); return; }
    if (req.method === 'GET' && url.pathname.startsWith('/v1/nodes/')) {
      const id = decodeURIComponent(url.pathname.slice('/v1/nodes/'.length));
      if (nodes[id]) { res.end(JSON.stringify(nodes[id])); return; }
      res.statusCode = 404;
      res.end('{"error":"not_ranked"}');
      return;
    }
    res.statusCode = 404;
    res.end('{}');
  });
  const port = await freePort();
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  const close = () => new Promise((r) => { server.closeAllConnections(); server.close(() => r()); });
  onCleanup(close);
  return { url: `http://127.0.0.1:${port}`, hits, close };
}

export function rankedNode(peerId) {
  return {
    peer_id: peerId, rank: 7, of: 42,
    availability: { d7: 0.99, d30: 0.97, d365: 0.9 },
    check_type: 'p2p_verified',
    attestation: { status: 'valid', expiry: 1800000000 },
    last_check_at: 1792000100,
  };
}
