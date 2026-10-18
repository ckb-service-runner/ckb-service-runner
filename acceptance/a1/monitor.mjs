// In-process mock of the CSR Network Monitor's check-in endpoints.
import http from 'node:http';
import { freePort, onCleanup } from './lib.mjs';
import { makeChallenge, newKeyPair } from './crypto.mjs';

export async function startMockMonitor(monKey, { height = 900, badSig = false } = {}) {
  const other = newKeyPair();
  const checkins = [];
  const answers = [];
  const issued = new Set();
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      let j = null;
      try { j = JSON.parse(body); } catch { /* keep null */ }
      res.setHeader('content-type', 'application/json');
      if (req.method === 'POST' && req.url === '/v1/checkin') {
        checkins.push(j);
        const c = makeChallenge(badSig ? other : monKey, { height });
        issued.add(c.nonce);
        res.end(JSON.stringify({ challenge: c }));
      } else if (req.method === 'POST' && req.url === '/v1/answer') {
        answers.push(j);
        res.end(JSON.stringify({ ok: true }));
      } else {
        res.statusCode = 404;
        res.end('{}');
      }
    });
  });
  const port = await freePort();
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  const close = () => new Promise((r) => { server.closeAllConnections(); server.close(() => r()); });
  onCleanup(close);
  return { port, url: `http://127.0.0.1:${port}`, checkins, answers, issued, close };
}
