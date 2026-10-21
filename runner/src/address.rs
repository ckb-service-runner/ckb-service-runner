//! CKB addresses: full format (`0x00 ‖ code_hash ‖ hash_type ‖ args`), bech32m. Prefix `ckb` on mainnet,
//! `ckt` on testnet and devnet. The old short and full (bech32) formats are refused.

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32M_CONST: u32 = 0x2bc8_30a3;
const GEN: [u32; 5] = [
    0x3b6a_57b2,
    0x2650_8e6d,
    0x1ea1_19fa,
    0x3d42_33dd,
    0x2a14_62b3,
];

/// Code hash of `secp256k1_blake160_sighash_all` (hash type `type`).
pub const SIGHASH_CODE_HASH: [u8; 32] = [
    0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9,
    0xfc, 0xc8, 0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8,
];

fn polymod(values: impl IntoIterator<Item = u8>) -> u32 {
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ v as u32;
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let b = hrp.as_bytes();
    let mut v: Vec<u8> = b.iter().map(|c| c >> 5).collect();
    v.push(0);
    v.extend(b.iter().map(|c| c & 31));
    v
}

/// Regroups bits (`from` → `to` bits per item); `pad` adds a final partial group.
fn convert_bits(data: &[u8], from: u32, to: u32, pad: bool) -> Option<Vec<u8>> {
    let (mut acc, mut bits) = (0u32, 0u32);
    let max = (1u32 << to) - 1;
    let mut out = Vec::new();
    for &d in data {
        if (d as u32) >> from != 0 {
            return None;
        }
        acc = (acc << from) | d as u32;
        bits += from;
        while bits >= to {
            bits -= to;
            out.push(((acc >> bits) & max) as u8);
        }
    }
    if pad {
        if bits > 0 {
            out.push(((acc << (to - bits)) & max) as u8);
        }
    } else if bits >= from || (acc << (to - bits)) & max != 0 {
        return None;
    }
    Some(out)
}

fn bech32m_encode(hrp: &str, payload: &[u8]) -> String {
    let words = convert_bits(payload, 8, 5, true).expect("8-bit input");
    let chk = polymod(
        hrp_expand(hrp)
            .into_iter()
            .chain(words.iter().copied())
            .chain([0; 6]),
    ) ^ BECH32M_CONST;
    let mut s = format!("{hrp}1");
    for w in words
        .iter()
        .copied()
        .chain((0..6).map(|i| ((chk >> (5 * (5 - i))) & 31) as u8))
    {
        s.push(CHARSET[w as usize] as char);
    }
    s
}

/// `(hrp, payload)` of a bech32m string; `None` if it is not one (bad checksum, mixed case, …).
fn bech32m_decode(s: &str) -> Option<(String, Vec<u8>)> {
    if s.bytes().any(|b| b.is_ascii_lowercase()) && s.bytes().any(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let s = s.to_ascii_lowercase();
    let pos = s.rfind('1')?;
    let (hrp, data) = (&s[..pos], &s[pos + 1..]);
    if hrp.is_empty() || data.len() < 6 || !hrp.bytes().all(|b| (33..=126).contains(&b)) {
        return None;
    }
    let words: Vec<u8> = data
        .bytes()
        .map(|c| CHARSET.iter().position(|&x| x == c).map(|p| p as u8))
        .collect::<Option<_>>()?;
    if polymod(hrp_expand(hrp).into_iter().chain(words.iter().copied())) != BECH32M_CONST {
        return None;
    }
    let payload = convert_bits(&words[..words.len() - 6], 5, 8, false)?;
    Some((hrp.to_string(), payload))
}

/// Address prefix of a network.
pub fn hrp(network: &str) -> &'static str {
    if network == "mainnet" {
        "ckb"
    } else {
        "ckt"
    }
}

/// A lock script as given by an address.
#[derive(Debug, Clone, PartialEq)]
pub struct LockScript {
    pub code_hash: [u8; 32],
    /// Molecule hash type byte: data 0, type 1, data1 2, data2 4.
    pub hash_type: u8,
    pub args: Vec<u8>,
}

/// Full-format address of a lock script.
pub fn encode(network: &str, lock: &LockScript) -> String {
    let mut payload = vec![0x00];
    payload.extend_from_slice(&lock.code_hash);
    payload.push(lock.hash_type);
    payload.extend_from_slice(&lock.args);
    bech32m_encode(hrp(network), &payload)
}

/// Address of the `secp256k1_blake160_sighash_all` lock (`type`) with these 20-byte args.
pub fn sighash_address(network: &str, args: &[u8]) -> String {
    encode(
        network,
        &LockScript {
            code_hash: SIGHASH_CODE_HASH,
            hash_type: 1,
            args: args.to_vec(),
        },
    )
}

/// Parses a full-format address of `network`.
pub fn decode(network: &str, addr: &str) -> Result<LockScript, String> {
    let addr = addr.trim();
    let (prefix, payload) = bech32m_decode(addr).ok_or_else(|| {
        format!("\"{addr}\" is not a CKB full-format address (bech32m, ckb1…/ckt1…)")
    })?;
    let want = hrp(network);
    if prefix != want {
        return Err(format!(
            "\"{addr}\" is an address of another network (prefix {prefix}, {network} uses {want})"
        ));
    }
    if payload.len() < 34 || payload[0] != 0x00 {
        return Err(format!(
            "\"{addr}\" is not a full-format address (only the 0x00 format is accepted)"
        ));
    }
    let hash_type = payload[33];
    if ![0u8, 1, 2, 4].contains(&hash_type) {
        return Err(format!("\"{addr}\" has an unknown hash type {hash_type}"));
    }
    Ok(LockScript {
        code_hash: payload[1..33].try_into().expect("32 bytes"),
        hash_type,
        args: payload[34..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEURON_ARGS: &str = "89c04afd2ded4725e915020cb757a20d353f1094";
    const NEURON_ADDR: &str = "ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsqvfcp906t0dguj7j9gzpjm40gsdx5l3p9qutm6yg";

    #[test]
    fn neuron_address() {
        let args = hex::decode(NEURON_ARGS).unwrap();
        assert_eq!(sighash_address("testnet", &args), NEURON_ADDR);
        assert_eq!(sighash_address("devnet", &args), NEURON_ADDR);
        let l = decode("testnet", NEURON_ADDR).unwrap();
        assert_eq!(
            (l.code_hash, l.hash_type, l.args),
            (SIGHASH_CODE_HASH, 1, args.clone())
        );
        let main = sighash_address("mainnet", &args);
        assert!(main.starts_with("ckb1q"));
        assert_eq!(decode("mainnet", &main).unwrap().args, args);
        assert_eq!(
            decode("testnet", &NEURON_ADDR.to_uppercase()).unwrap().args,
            args
        );
    }

    #[test]
    fn refusals() {
        assert!(decode("mainnet", NEURON_ADDR)
            .unwrap_err()
            .contains("another network"));
        let mut bad = NEURON_ADDR.to_string();
        let last = bad.pop().unwrap();
        bad.push(if last == 'q' { 'p' } else { 'q' });
        assert!(decode("testnet", &bad).is_err());
        assert!(decode("testnet", &format!("0x{NEURON_ARGS}")).is_err());
        // the deprecated short format (0x01, bech32): ckt1qyq… of the same args
        assert!(decode("testnet", "ckt1qyqgnszwl5k763e9ay2syr9h273q6dflzz2qp2hhzd").is_err());
        let mixed = format!(
            "{}{}",
            &NEURON_ADDR[..10].to_uppercase(),
            &NEURON_ADDR[10..]
        );
        assert!(decode("testnet", &mixed).is_err());
        // a valid bech32m string with a non-full-format payload
        assert!(decode("testnet", &bech32m_encode("ckt", &[1, 2, 3])).is_err());
    }
}
