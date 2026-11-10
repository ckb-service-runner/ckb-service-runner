// CSR job cell builder: molecule `Job` encoding, job cell, cell deps, watch payload, service ID.
// Plain ES module, no imports; browsers and Node >= 18. u64 values are BigInt.

const M64 = (1n << 64n) - 1n;
const SHANNONS_PER_CKB = 100000000n;
const TRIGGERS = ['OnDemand', 'AtBlock', 'AtEpoch', 'AtTimestamp', 'Every'];
const TRIGGER_FIELD = { AtBlock: 'block', AtEpoch: 'epoch', AtTimestamp: 'timestampMs', Every: 'blocks' };

// ---- hex helpers ------------------------------------------------------------------------------

function hexToBytes(hex, what = 'hex') {
  if (typeof hex !== 'string' || !/^0x([0-9a-fA-F]{2})*$/.test(hex)) throw new Error(`${what}: expected 0x-prefixed even-length hex`);
  const out = new Uint8Array((hex.length - 2) / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(2 + 2 * i, 4 + 2 * i), 16);
  return out;
}

function bytesToHex(b) {
  let s = '0x';
  for (const x of b) s += x.toString(16).padStart(2, '0');
  return s;
}

function hash32(hex, what) {
  const b = hexToBytes(hex, what);
  if (b.length !== 32) throw new Error(`${what}: expected 32 bytes`);
  return b;
}

function concat(parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

function u32(n) {
  if (!Number.isInteger(n) || n < 0 || n > 0xffffffff) throw new Error('u32 out of range');
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, n, true);
  return b;
}

function u64(v, what) {
  if (typeof v !== 'bigint') throw new Error(`${what}: expected BigInt`);
  if (v < 0n || v > M64) throw new Error(`${what}: out of u64 range`);
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return b;
}

// ---- molecule ---------------------------------------------------------------------------------

function table(fields) {
  const header = 4 + 4 * fields.length;
  const parts = [u32(header + fields.reduce((n, f) => n + f.length, 0))];
  let off = header;
  for (const f of fields) { parts.push(u32(off)); off += f.length; }
  return concat([...parts, ...fields]);
}

/** Splits a molecule table into its fields; strict about header, offsets and size. */
function tableFields(d, count) {
  const dv = new DataView(d.buffer, d.byteOffset, d.length);
  if (d.length < 4 + 4 * count || dv.getUint32(0, true) !== d.length) throw new Error('malformed table');
  const offs = [];
  for (let i = 0; i < count; i++) offs.push(dv.getUint32(4 + 4 * i, true));
  if (count === 0) return [];
  if (offs[0] !== 4 + 4 * count) throw new Error('malformed table header');
  offs.push(d.length);
  const out = [];
  for (let i = 0; i < count; i++) {
    if (offs[i] > offs[i + 1]) throw new Error('malformed table offsets');
    out.push(d.subarray(offs[i], offs[i + 1]));
  }
  return out;
}

function readU64(b, what) {
  if (b.length !== 8) throw new Error(`${what}: expected 8 bytes`);
  return new DataView(b.buffer, b.byteOffset, 8).getBigUint64(0, true);
}

function encodeTrigger(t) {
  const id = TRIGGERS.indexOf(t && t.kind);
  if (id < 0) throw new Error('unknown trigger kind');
  const inner = id === 0 ? table([]) : table([u64(t[TRIGGER_FIELD[t.kind]], t.kind)]);
  return concat([u32(id), inner]);
}

function decodeTrigger(d) {
  if (d.length < 4) throw new Error('malformed trigger');
  const id = new DataView(d.buffer, d.byteOffset, d.length).getUint32(0, true);
  if (id >= TRIGGERS.length) throw new Error('unknown trigger id');
  const kind = TRIGGERS[id];
  const inner = d.subarray(4);
  if (id === 0) {
    if (inner.length !== 4 || tableFields(inner, 0).length !== 0) throw new Error('malformed OnDemand');
    return { kind };
  }
  return { kind, [TRIGGER_FIELD[kind]]: readU64(tableFields(inner, 1)[0], kind) };
}

/** Molecule `Job` as a 0x hex string. */
export function encodeJob(job) {
  const payload = hexToBytes(job.payload ?? '0x', 'payload');
  return bytesToHex(table([
    encodeTrigger(job.trigger),
    u64(job.tipShannons, 'tipShannons'),
    u64(job.maxFeeShannons, 'maxFeeShannons'),
    u64(job.maxCycles, 'maxCycles'),
    concat([u32(payload.length), payload]),
    u64(job.expiryBlock, 'expiryBlock'),
    hash32(job.refundLockHash, 'refundLockHash'),
    u32(job.remainingRuns),
  ]));
}

/** Inverse of `encodeJob`; throws on malformed data. */
export function decodeJob(hex) {
  const f = tableFields(hexToBytes(hex, 'job data'), 8);
  const plen = f[4].length >= 4 ? new DataView(f[4].buffer, f[4].byteOffset, f[4].length).getUint32(0, true) : -1;
  if (plen !== f[4].length - 4) throw new Error('malformed payload');
  if (f[6].length !== 32 || f[7].length !== 4) throw new Error('malformed job fields');
  return {
    trigger: decodeTrigger(f[0]),
    tipShannons: readU64(f[1], 'tipShannons'),
    maxFeeShannons: readU64(f[2], 'maxFeeShannons'),
    maxCycles: readU64(f[3], 'maxCycles'),
    payload: bytesToHex(f[4].subarray(4)),
    expiryBlock: readU64(f[5], 'expiryBlock'),
    refundLockHash: bytesToHex(f[6]),
    remainingRuns: new DataView(f[7].buffer, f[7].byteOffset, 4).getUint32(0, true),
  };
}

// ---- deployment -------------------------------------------------------------------------------

/** Accepts the content of `deployments/<network>.json` (string or object) and checks the fields used here. */
export function loadDeployment(json) {
  const d = typeof json === 'string' ? JSON.parse(json) : json;
  if (!d || typeof d !== 'object') throw new Error('deployment: not an object');
  hash32(d.registry_type_hash, 'deployment.registry_type_hash');
  for (const name of ['job_type', 'job_lock']) {
    const s = d.scripts && d.scripts[name];
    if (!s) throw new Error(`deployment.scripts.${name} missing`);
    hash32(s.code_hash, `${name}.code_hash`);
    hash32(s.tx_hash, `${name}.tx_hash`);
    if (!Number.isInteger(s.index) || s.index < 0) throw new Error(`${name}.index invalid`);
    if (s.hash_type !== 'data2') throw new Error(`${name}.hash_type must be data2`);
  }
  return d;
}

// ---- job cell ---------------------------------------------------------------------------------

/** Minimum capacity of a job cell (`csr-job-type` creation rule): occupied capacity + runs × (tip + max fee). */
export function minJobCapacity({ deployment, serviceId: sid, job }) {
  loadDeployment(deployment);
  hash32(sid, 'serviceId');
  const dataLen = BigInt(hexToBytes(encodeJob(job)).length);
  // 8 capacity + lock (33 + 32 args) + type (33 + 32 args) + data
  const occupied = (8n + 65n + 65n + dataLen) * SHANNONS_PER_CKB;
  return occupied + BigInt(job.remainingRuns) * (job.tipShannons + job.maxFeeShannons);
}

/** `{ cellOutput: { capacity, lock, type }, outputData }`, CCC-compatible plain objects. */
export function jobCell({ deployment, serviceId: sid, job, extraCapacityShannons = 0n }) {
  const d = loadDeployment(deployment);
  const capacity = minJobCapacity({ deployment: d, serviceId: sid, job }) + extraCapacityShannons;
  return {
    cellOutput: {
      capacity: '0x' + capacity.toString(16),
      lock: { codeHash: d.scripts.job_lock.code_hash, hashType: 'data2', args: d.registry_type_hash },
      type: { codeHash: d.scripts.job_type.code_hash, hashType: 'data2', args: sid },
    },
    outputData: encodeJob(job),
  };
}

/** Cell deps for a transaction that creates (or spends) a job cell. */
export function jobCellDeps(deployment) {
  const d = loadDeployment(deployment);
  return ['job_type', 'job_lock'].map((n) => ({
    outPoint: { txHash: d.scripts[n].tx_hash, index: d.scripts[n].index },
    depType: 'code',
  }));
}

/** Watch payload: "CSRW" ‖ 0x01 ‖ tx hash ‖ index u32 LE ‖ inner. `outPoint` is `{ txHash, index }`. */
export function watchPayload(outPoint, innerHex = '0x') {
  return bytesToHex(concat([
    new Uint8Array([0x43, 0x53, 0x52, 0x57, 1]),
    hash32(outPoint.txHash, 'txHash'),
    u32(outPoint.index),
    hexToBytes(innerHex, 'inner'),
  ]));
}

// ---- blake2b-256 (ckb-default-hash) -----------------------------------------------------------

const IV = [0x6a09e667f3bcc908n, 0xbb67ae8584caa73bn, 0x3c6ef372fe94f82bn, 0xa54ff53a5f1d36f1n,
  0x510e527fade682d1n, 0x9b05688c2b3e6c1fn, 0x1f83d9abfb41bd6bn, 0x5be0cd19137e2179n];
const SIGMA = [
  [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
  [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4], [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
  [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13], [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
  [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11], [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
  [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5], [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];
const rotr = (x, n) => ((x >> n) | (x << (64n - n))) & M64;

function compress(h, block, t, last) {
  const dv = new DataView(block.buffer, block.byteOffset, 128);
  const m = Array.from({ length: 16 }, (_, i) => dv.getBigUint64(8 * i, true));
  const v = [...h, ...IV];
  v[12] ^= t & M64;
  v[13] ^= t >> 64n;
  if (last) v[14] ^= M64;
  const g = (a, b, c, d, x, y) => {
    v[a] = (v[a] + v[b] + x) & M64; v[d] = rotr(v[d] ^ v[a], 32n);
    v[c] = (v[c] + v[d]) & M64; v[b] = rotr(v[b] ^ v[c], 24n);
    v[a] = (v[a] + v[b] + y) & M64; v[d] = rotr(v[d] ^ v[a], 16n);
    v[c] = (v[c] + v[d]) & M64; v[b] = rotr(v[b] ^ v[c], 63n);
  };
  for (let r = 0; r < 12; r++) {
    const s = SIGMA[r % 10];
    g(0, 4, 8, 12, m[s[0]], m[s[1]]); g(1, 5, 9, 13, m[s[2]], m[s[3]]);
    g(2, 6, 10, 14, m[s[4]], m[s[5]]); g(3, 7, 11, 15, m[s[6]], m[s[7]]);
    g(0, 5, 10, 15, m[s[8]], m[s[9]]); g(1, 6, 11, 12, m[s[10]], m[s[11]]);
    g(2, 7, 8, 13, m[s[12]], m[s[13]]); g(3, 4, 9, 14, m[s[14]], m[s[15]]);
  }
  for (let i = 0; i < 8; i++) h[i] ^= v[i] ^ v[i + 8];
}

/** blake2b with 32-byte output and the CKB personalization "ckb-default-hash"; `data` is a Uint8Array. */
export function blake2b256(data) {
  const person = new TextEncoder().encode('ckb-default-hash');
  const pdv = new DataView(person.buffer);
  const h = [...IV];
  h[0] ^= 0x01010020n;
  h[6] ^= pdv.getBigUint64(0, true);
  h[7] ^= pdv.getBigUint64(8, true);
  let off = 0;
  while (data.length - off > 128) {
    compress(h, data.subarray(off, off + 128), BigInt(off + 128), false);
    off += 128;
  }
  const last = new Uint8Array(128);
  last.set(data.subarray(off));
  compress(h, last, BigInt(data.length), true);
  const out = new Uint8Array(32);
  const odv = new DataView(out.buffer);
  for (let i = 0; i < 4; i++) odv.setBigUint64(8 * i, h[i], true);
  return out;
}

/** Service ID (D49 of spec/service-manifest.md). Plain code hash when there are no args and no data cells. */
export function serviceId(codeHash, argsHex = '0x', dataHashes = []) {
  const code = hash32(codeHash, 'codeHash');
  const args = hexToBytes(argsHex, 'args');
  if (args.length === 0 && dataHashes.length === 0) return bytesToHex(code);
  if (dataHashes.length > 255) throw new Error('too many data hashes');
  return bytesToHex(blake2b256(concat([
    new TextEncoder().encode('CSR-SERVICE-v1'), code, u32(args.length), args,
    new Uint8Array([dataHashes.length]), ...dataHashes.map((h, i) => hash32(h, `dataHashes[${i}]`)),
  ])));
}
