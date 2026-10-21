// CKB full-format addresses (bech32m) and the stake consent text, for the A8 tests.
const CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
const GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
function polymod(values) {
  let chk = 1;
  for (const v of values) {
    const b = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((b >>> i) & 1) chk ^= GEN[i];
  }
  return chk >>> 0;
}
const hrpExpand = (h) => [...[...h].map((c) => c.charCodeAt(0) >> 5), 0, ...[...h].map((c) => c.charCodeAt(0) & 31)];
function toWords(bytes) {
  let acc = 0; let bits = 0; const out = [];
  for (const b of bytes) {
    acc = (acc << 8) | b; bits += 8;
    while (bits >= 5) { bits -= 5; out.push((acc >>> bits) & 31); }
  }
  if (bits > 0) out.push((acc << (5 - bits)) & 31);
  return out;
}
export const SIGHASH_CODE_HASH = '9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8';

// Full address (0x00 format) of the secp256k1_blake160 lock with these 20-byte args.
export function secpAddress(lockArgs, network) {
  const hrp = network === 'mainnet' ? 'ckb' : 'ckt';
  const payload = Buffer.from('00' + SIGHASH_CODE_HASH + '01' + lockArgs.replace(/^0x/, ''), 'hex');
  const words = toWords(payload);
  const chk = polymod([...hrpExpand(hrp), ...words, 0, 0, 0, 0, 0, 0]) ^ 0x2bc830a3;
  const check = [0, 1, 2, 3, 4, 5].map((i) => (chk >>> (5 * (5 - i))) & 31);
  return hrp + '1' + [...words, ...check].map((w) => CHARSET[w]).join('');
}

// The text the deposit owner signs (Neuron "Sign message"): one line, fields separated by "|".
export const consentText = ({ network, stake, servicePubkey, peerId }) =>
  `CSR-STAKE-CONSENT-v1|${network}|${stake.toLowerCase()}|${servicePubkey.toLowerCase()}|${peerId}`;

// Neuron vector (testnet wallet): message "Message from CSR" signed by lock args 0x89c0…1094.
export const NEURON = {
  message: 'Message from CSR',
  signature: '0xc0fee686c87c2f398a83ef0ab8ddcc462b0acd552411674b3d1cf8e9ce6a86563204ffd3e2b82a6dfa9be40c58e1d670faf8ea8656db45e1d05d4265912877f401',
  lock_args: '0x89c04afd2ded4725e915020cb757a20d353f1094',
  address: 'ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqvfcp906t0dguj7j9gzpjm40gsdx5l3p9qutm6yg',
};
