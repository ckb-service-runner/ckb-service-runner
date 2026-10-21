//! Stake consent (`spec/registry.md` §4.2): the text the stake owner signs in its wallet and the recovery of
//! the signer. Text and digest come from the contract's own `consent.rs`.

#[allow(dead_code)]
#[path = "../../contracts/csr-registry/src/consent.rs"]
mod shared;

use secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
use secp256k1::{Message, Secp256k1};

/// The consent text for `stake` (lower-case `0x<tx>:<index>`), the service key (33 bytes) and the peer id.
pub fn message(
    network: &str,
    stake: &[u8; 36],
    service_pubkey: &[u8],
    peer_id: &str,
) -> Result<String, String> {
    let net = match network {
        "mainnet" => 0,
        "testnet" => 1,
        "devnet" => 2,
        _ => return Err(format!("unknown network {network}")),
    };
    let pk: &[u8; 33] = service_pubkey
        .try_into()
        .map_err(|_| "the service key must be 33 bytes".to_string())?;
    let mut text = Vec::new();
    shared::consent_text(net, stake, pk, peer_id.as_bytes(), &mut |p| {
        text.extend_from_slice(p)
    })
    .ok_or("unknown network")?;
    String::from_utf8(text).map_err(|_| "the peer id is not UTF-8".to_string())
}

/// Parses `0x` + 130 hex digits (65 bytes `r ‖ s ‖ v`).
pub fn parse_signature_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    let h = s
        .strip_prefix("0x")
        .ok_or("the signature must start with 0x (65 bytes r ‖ s ‖ v in hex)")?;
    let b = hex::decode(h).map_err(|_| "the signature is not hex".to_string())?;
    if b.len() != 65 {
        return Err(format!(
            "the signature must be 65 bytes (r ‖ s ‖ v), got {}",
            b.len()
        ));
    }
    Ok(b)
}

/// `blake160` of the compressed key that signed `text` ("Sign message" format of Neuron).
pub fn recover_signer(text: &str, signature: &[u8]) -> Result<[u8; 20], String> {
    let (rs, recid) = shared::parse_signature(signature)
        .ok_or("malformed signature: expected 65 bytes r ‖ s ‖ v with v 0–3 or 27–30")?;
    let digest = shared::message_digest(text.as_bytes());
    let key = RecoveryId::from_i32(recid as i32)
        .and_then(|rec| RecoverableSignature::from_compact(&rs, rec))
        .and_then(|sig| {
            Secp256k1::verification_only().recover_ecdsa(&Message::from_digest(digest), &sig)
        })
        .map_err(|_| "invalid signature: no public key can be recovered from it".to_string())?;
    let h = ckb_hash::blake2b_256(key.serialize());
    Ok(h[..20].try_into().expect("20 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEURON_SIG: &str = "0xc0fee686c87c2f398a83ef0ab8ddcc462b0acd552411674b3d1cf8e9ce6a86563204ffd3e2b82a6dfa9be40c58e1d670faf8ea8656db45e1d05d4265912877f401";

    #[test]
    fn text_vector() {
        let mut stake = [0x5au8; 36];
        stake[32..].copy_from_slice(&3u32.to_le_bytes());
        let pk = hex::decode("02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27")
            .unwrap();
        let t = message(
            "testnet",
            &stake,
            &pk,
            "QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB",
        )
        .unwrap();
        assert_eq!(t, format!("CSR-STAKE-CONSENT-v1|testnet|0x{}:3|0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27|QmTestPeer1234567890abcdefghijklmnopqrstuvwxyzAB", "5a".repeat(32)));
        assert_eq!(
            hex::encode(shared::message_digest(t.as_bytes())),
            "e3425937793f1e3c6de997e908df0c8ac00efdfd8db48948adca4d4e9a4b51c2"
        );
    }

    #[test]
    fn neuron_vector() {
        let sig = parse_signature_hex(NEURON_SIG).unwrap();
        assert_eq!(
            hex::encode(recover_signer("Message from CSR", &sig).unwrap()),
            "89c04afd2ded4725e915020cb757a20d353f1094"
        );
        // v + 27 is the same signature
        let mut v27 = sig.clone();
        v27[64] += 27;
        assert_eq!(
            recover_signer("Message from CSR", &v27).unwrap(),
            recover_signer("Message from CSR", &sig).unwrap()
        );
        let mut bad = sig.clone();
        bad[64] = 5;
        assert!(recover_signer("Message from CSR", &bad)
            .unwrap_err()
            .contains("signature"));
        assert!(parse_signature_hex("0x1234")
            .unwrap_err()
            .contains("signature"));
        assert!(parse_signature_hex(&NEURON_SIG[2..]).is_err());
    }
}
