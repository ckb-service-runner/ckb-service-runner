// Fake CSR Network Monitor answering POST /v1/attest.
import http from 'node:http';
import { freePort, onCleanup } from '../a1/lib.mjs';

// reply(body) returns [status, json]
export async function startAttestMonitor(reply) {
  const requests = [];
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      let j = null;
      try { j = JSON.parse(body); } catch { /* keep null */ }
      res.setHeader('content-type', 'application/json');
      if (req.method === 'POST' && req.url === '/v1/attest') {
        requests.push(j);
        const [status, out] = reply(j);
        res.statusCode = status;
        res.end(JSON.stringify(out));
      } else {
        res.statusCode = 404;
        res.end('{"error":"not_found"}');
      }
    });
  });
  const port = await freePort();
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  const close = () => new Promise((r) => { server.closeAllConnections(); server.close(() => r()); });
  onCleanup(close);
  return { port, url: `http://127.0.0.1:${port}`, requests, close };
}
