// Small CKB helpers for the A7 tests: blake2b-256 with the CKB personalization (via python3, which node's
// crypto cannot do) and molecule script hashes.
import { spawnSync } from 'node:child_process';

export function ckbHash(hex) {
  const h = hex.startsWith('0x') ? hex.slice(2) : hex;
  const r = spawnSync('python3', ['-I', '-c',
    "import hashlib,sys;print('0x'+hashlib.blake2b(bytes.fromhex(sys.argv[1]),digest_size=32,person=b'ckb-default-hash').hexdigest())", h],
  { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`python3 failed: ${r.stderr}`);
  return r.stdout.trim();
}

export function fileHash(file) {
  const r = spawnSync('python3', ['-I', '-c',
    "import hashlib,sys;print('0x'+hashlib.blake2b(open(sys.argv[1],'rb').read(),digest_size=32,person=b'ckb-default-hash').hexdigest())", file],
  { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`python3 failed: ${r.stderr}`);
  return r.stdout.trim();
}

const u32 = (n) => Buffer.from([n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff]).toString('hex');
const HASH_TYPES = { data: 0, type: 1, data1: 2, data2: 4 };

// molecule Script table: total size, 3 offsets, code_hash (32), hash_type (1), args (Bytes)
export function scriptBytes({ code_hash, hash_type, args }) {
  const a = args.replace(/^0x/, '');
  const argsMol = u32(a.length / 2) + a;
  const total = 16 + 32 + 1 + argsMol.length / 2;
  return u32(total) + u32(16) + u32(48) + u32(49) + code_hash.replace(/^0x/, '') +
    HASH_TYPES[hash_type].toString(16).padStart(2, '0') + argsMol;
}

export const scriptHash = (s) => ckbHash(scriptBytes(s));
