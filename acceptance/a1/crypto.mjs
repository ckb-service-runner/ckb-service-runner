// secp256k1 helpers for the ping protocol tests (ECDSA over SHA-256, compact r||s signatures).
import crypto from 'node:crypto';

const P = 0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2fn;
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
const strip = (h) => (h.startsWith('0x') ? h.slice(2) : h);
function modpow(b, e, m) {
  let r = 1n;
  b %= m;
  while (e > 0n) {
    if (e & 1n) r = (r * b) % m;
    b = (b * b) % m;
    e >>= 1n;
  }
  return r;
}

export function newKeyPair() {
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ec', { namedCurve: 'secp256k1' });
  const jwk = publicKey.export({ format: 'jwk' });
  const x = Buffer.from(jwk.x, 'base64url');
  const y = Buffer.from(jwk.y, 'base64url');
  const compressed = Buffer.concat([Buffer.from([(y[y.length - 1] & 1) ? 3 : 2]), x]);
  return { privateKey, publicKey, pubHex: '0x' + compressed.toString('hex') };
}

export function pubFromCompressed(pubHex) {
  const b = Buffer.from(strip(pubHex), 'hex');
  if (b.length !== 33 || (b[0] !== 2 && b[0] !== 3)) throw new Error(`not a compressed public key: ${pubHex}`);
  const x = BigInt('0x' + b.subarray(1).toString('hex'));
  let y = modpow((x ** 3n + 7n) % P, (P + 1n) / 4n, P);
  if ((y & 1n) !== BigInt(b[0] & 1)) y = P - y;
  const yb = Buffer.from(y.toString(16).padStart(64, '0'), 'hex');
  return crypto.createPublicKey({
    key: { kty: 'EC', crv: 'secp256k1', x: b.subarray(1).toString('base64url'), y: yb.toString('base64url') },
    format: 'jwk',
  });
}

export function pubFromPrivHex(privHex) {
  const e = crypto.createECDH('secp256k1');
  e.setPrivateKey(Buffer.from(strip(privHex.trim()), 'hex'));
  return '0x' + e.getPublicKey('hex', 'compressed');
}

export function sign(privateKey, message) {
  return '0x' + crypto.sign('sha256', Buffer.from(message, 'utf8'), { key: privateKey, dsaEncoding: 'ieee-p1363' }).toString('hex');
}

export function verify(pubHex, message, sigHex) {
  if (typeof sigHex !== 'string') return false;
  const sig = Buffer.from(strip(sigHex), 'hex');
  if (sig.length !== 64) return false;
  return crypto.verify('sha256', Buffer.from(message, 'utf8'), { key: pubFromCompressed(pubHex), dsaEncoding: 'ieee-p1363' }, sig);
}

export function isLowS(sigHex) {
  return BigInt('0x' + strip(sigHex).slice(64)) <= N / 2n;
}

export function toHighS(sigHex) {
  const h = strip(sigHex);
  let s = BigInt('0x' + h.slice(64));
  if (s <= N / 2n) s = N - s;
  return '0x' + h.slice(0, 64) + s.toString(16).padStart(64, '0');
}

export const challengeMsg = (c) => `CSR-PING-CHALLENGE-v1\n${c.nonce}\n${c.height}\n${c.issued_at}`;
export const bindingMsg = (peerId, pub, network) => `CSR-BINDING-v1\n${peerId}\n${pub}\n${network}`;
export const ANSWER_FIELDS = ['nonce', 'peer_id', 'service_pubkey', 'binding_sig', 'network', 'csr_version', 'node_version',
  'r2_state', 'tip_number', 'tip_hash', 'block_hash_at_height', 'uptime_s', 'ping_mode'];
export const answerMsg = (a) => ['CSR-PING-ANSWER-v1', ...ANSWER_FIELDS.map((f) => String(a[f]))].join('\n');

export function makeChallenge(key, { height, issuedAt = Math.floor(Date.now() / 1000), nonce } = {}) {
  const c = { nonce: nonce || '0x' + crypto.randomBytes(32).toString('hex'), height, issued_at: issuedAt };
  c.monitor_sig = sign(key.privateKey, challengeMsg(c));
  return c;
}
