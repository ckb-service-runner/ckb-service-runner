//! Stake consent (`spec/registry.md` §4.2): the text the stake owner signs, its digest (Neuron's "Sign message"
//! format) and the signature format. `no_std`, no allocation; shared with the contract tests via `#[path]`.
//!
//! Text: `CSR-STAKE-CONSENT-v1|<network>|0x<stake tx hash>:<stake index>|0x<service_pubkey>|<peer_id>`.
//! Digest: `blake2b-256("Nervos Message:" ‖ text)`, personalization `ckb-default-hash`.

/// First field of the consent text.
pub const CONSENT_PREFIX: &[u8] = b"CSR-STAKE-CONSENT-v1";
/// Prefix Neuron puts before a message it signs (no space after the colon).
pub const MESSAGE_MAGIC: &[u8] = b"Nervos Message:";

/// Order `n` of the secp256k1 group (big endian).
const N: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6,
    0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
];
/// `n / 2` (big endian): a signature with `s > n / 2` is high-S.
const HALF_N: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x5d, 0x57, 0x6e, 0x73,
    0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
];

/// Network name of the registry `network` byte (0 mainnet, 1 testnet, 2 devnet).
pub fn network_name(network: u8) -> Option<&'static [u8]> {
    match network {
        0 => Some(b"mainnet"),
        1 => Some(b"testnet"),
        2 => Some(b"devnet"),
        _ => None,
    }
}

fn hex_into<'a>(bytes: &[u8], out: &'a mut [u8]) -> &'a [u8] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (i, b) in bytes.iter().enumerate() {
        out[2 * i] = HEX[(b >> 4) as usize];
        out[2 * i + 1] = HEX[(b & 15) as usize];
    }
    &out[..2 * bytes.len()]
}

fn decimal_into(mut v: u32, out: &mut [u8; 10]) -> &[u8] {
    let mut i = out.len();
    loop {
        i -= 1;
        out[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            return &out[i..];
        }
    }
}

/// Emits the consent text piece by piece to `sink`. `stake` is the molecule `OutPoint`
/// (`tx_hash (32) ‖ index (u32 LE)`), `peer_id` the raw peer id bytes. `None` for an unknown network.
pub fn consent_text(network: u8, stake: &[u8; 36], service_pubkey: &[u8; 33], peer_id: &[u8], sink: &mut dyn FnMut(&[u8])) -> Option<()> {
    let name = network_name(network)?;
    let mut tx_hash = [0u8; 64];
    let mut index = [0u8; 10];
    let mut pubkey = [0u8; 66];
    let idx = u32::from_le_bytes([stake[32], stake[33], stake[34], stake[35]]);
    let parts: [&[u8]; 11] = [
        CONSENT_PREFIX,
        b"|",
        name,
        b"|0x",
        hex_into(&stake[..32], &mut tx_hash),
        b":",
        decimal_into(idx, &mut index),
        b"|0x",
        hex_into(service_pubkey, &mut pubkey),
        b"|",
        peer_id,
    ];
    for p in parts {
        sink(p);
    }
    Some(())
}

fn hasher() -> blake2b_ref::Blake2b {
    blake2b_ref::Blake2bBuilder::new(32).personal(b"ckb-default-hash").build()
}

/// Digest Neuron signs for a message: `blake2b-256("Nervos Message:" ‖ text)`.
#[allow(dead_code)] // used by the contract tests (Neuron vector)
pub fn message_digest(text: &[u8]) -> [u8; 32] {
    let mut h = hasher();
    h.update(MESSAGE_MAGIC);
    h.update(text);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

/// Digest of the consent text of these registry fields (`None` for an unknown network).
pub fn consent_digest(network: u8, stake: &[u8; 36], service_pubkey: &[u8; 33], peer_id: &[u8]) -> Option<[u8; 32]> {
    let mut h = hasher();
    h.update(MESSAGE_MAGIC);
    consent_text(network, stake, service_pubkey, peer_id, &mut |p| h.update(p))?;
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    Some(out)
}

/// Parses a 65-byte `r ‖ s ‖ v` signature: `v` 0–3 or 27–30 (minus 27). A high-S signature is normalised to
/// `s → n − s` with the low bit of the recovery id flipped. Returns `(r ‖ s (low-S), recovery id 0–3)`.
/// Range checks of `r` and `s` (non-zero, below `n`) are left to the signature library.
pub fn parse_signature(sig: &[u8]) -> Option<([u8; 64], u8)> {
    if sig.len() != 65 {
        return None;
    }
    let mut recid = match sig[64] {
        v @ 0..=3 => v,
        v @ 27..=30 => v - 27,
        _ => return None,
    };
    let mut rs = [0u8; 64];
    rs.copy_from_slice(&sig[..64]);
    let s: [u8; 32] = rs[32..].try_into().ok()?;
    if s > HALF_N && s < N {
        // n − s (big endian subtraction; s < n so there is no final borrow)
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let d = N[i] as i16 - s[i] as i16 - borrow;
            borrow = (d < 0) as i16;
            rs[32 + i] = (d + 256 * borrow) as u8;
        }
        recid ^= 1;
    }
    Some((rs, recid))
}
