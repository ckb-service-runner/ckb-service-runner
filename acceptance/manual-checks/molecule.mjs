// Minimal molecule encoding of a CKB Transaction whose skeleton only has inputs (skeleton guard check).
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const u64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };

function table(fields) {
  const header = 4 + 4 * fields.length;
  const offsets = [];
  let at = header;
  for (const f of fields) { offsets.push(u32(at)); at += f.length; }
  return Buffer.concat([u32(at), ...offsets, ...fields]);
}
const fixvec = (items) => Buffer.concat([u32(items.length), ...items]);
const emptyDynvec = () => u32(4);

// "0x<tx hash>:<index>" → CellInput (since 0).
function cellInput(outpoint) {
  const [h, i] = outpoint.split(':');
  return Buffer.concat([u64(0), Buffer.from(h.replace(/^0x/, ''), 'hex'), u32(Number(i))]);
}

// A Transaction with these inputs and nothing else, as 0x-hex.
export function skeletonSpending(outpoints) {
  const raw = table([u32(0), fixvec([]), fixvec([]), fixvec(outpoints.map(cellInput)), emptyDynvec(), emptyDynvec()]);
  return '0x' + table([raw, emptyDynvec()]).toString('hex');
}

// The watch header of the cell-consumed trigger: "CSRW" ‖ 1 ‖ out point (tx hash ‖ index u32 LE) ‖ inner.
export function watchPayload(outpoint, innerHex = '0x') {
  const [h, i] = outpoint.split(':');
  return '0x' + Buffer.concat([Buffer.from('CSRW'), Buffer.from([1]), Buffer.from(h.replace(/^0x/, ''), 'hex'), u32(Number(i)),
    Buffer.from(innerHex.replace(/^0x/, ''), 'hex')]).toString('hex');
}
