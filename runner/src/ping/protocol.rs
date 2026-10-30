//! Ping protocol v1.1: message builders, signing and verification (see `spec/ping-protocol.md`).

use secp256k1::{ecdsa::Signature, Message, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FRESHNESS_SECS: i64 = 60;
pub const REPLAY_WINDOW_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub nonce: String,
    pub height: u64,
    pub issued_at: i64,
    pub monitor_sig: String,
}

/// The 13 signed answer fields, in signing order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    pub nonce: String,
    pub peer_id: String,
    pub service_pubkey: String,
    pub binding_sig: String,
    pub network: String,
    pub csr_version: String,
    pub node_version: String,
    pub r2_state: String,
    pub tip_number: u64,
    pub tip_hash: String,
    pub block_hash_at_height: String,
    pub uptime_s: u64,
    pub ping_mode: String,
}

pub fn challenge_message(nonce: &str, height: u64, issued_at: i64) -> String {
    format!("CSR-PING-CHALLENGE-v1\n{nonce}\n{height}\n{issued_at}")
}

pub fn binding_message(peer_id: &str, service_pubkey: &str, network: &str) -> String {
    format!("CSR-BINDING-v1\n{peer_id}\n{service_pubkey}\n{network}")
}

/// The message the service key signs in a check-in (`ts` in unix seconds).
pub fn checkin_message(peer_id: &str, service_pubkey: &str, network: &str, ts: i64) -> String {
    format!("CSR-CHECKIN-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}")
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChallengeStatus {
    Answer { block_known: bool },
    ClockSkew,
}

/// `ClockSkew` when |now - issued_at| > FRESHNESS_SECS; otherwise answer, with the block known when height <= tip.
pub fn challenge_status(tip: u64, height: u64, issued_at: i64, now: i64) -> ChallengeStatus {
    if (now - issued_at).abs() > FRESHNESS_SECS {
        ChallengeStatus::ClockSkew
    } else {
        ChallengeStatus::Answer {
            block_known: height <= tip,
        }
    }
}

pub fn answer_message(a: &Answer) -> String {
    [
        "CSR-PING-ANSWER-v1",
        &a.nonce,
        &a.peer_id,
        &a.service_pubkey,
        &a.binding_sig,
        &a.network,
        &a.csr_version,
        &a.node_version,
        &a.r2_state,
        &a.tip_number.to_string(),
        &a.tip_hash,
        &a.block_hash_at_height,
        &a.uptime_s.to_string(),
        &a.ping_mode,
    ]
    .join("\n")
}

fn digest(message: &str) -> Message {
    Message::from_digest(Sha256::digest(message.as_bytes()).into())
}

/// Signs SHA-256(message); the result is a low-S compact signature as `0x` + 128 hex.
pub fn sign(key: &SecretKey, message: &str) -> String {
    let sig = Secp256k1::signing_only().sign_ecdsa(&digest(message), key);
    format!("0x{}", hex::encode(sig.serialize_compact()))
}

/// Verifies a compact signature after normalising S (high-S input is accepted).
pub fn verify(pubkey: &PublicKey, message: &str, sig_hex: &str) -> bool {
    let Some(h) = sig_hex.strip_prefix("0x") else {
        return false;
    };
    let Ok(bytes) = hex::decode(h) else {
        return false;
    };
    let Ok(mut sig) = Signature::from_compact(&bytes) else {
        return false;
    };
    sig.normalize_s();
    Secp256k1::verification_only()
        .verify_ecdsa(&digest(message), &sig, pubkey)
        .is_ok()
}

/// `0x` + 64 lowercase hex chars.
pub fn valid_nonce(n: &str) -> bool {
    n.len() == 66
        && n.starts_with("0x")
        && n[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> SecretKey {
        SecretKey::from_slice(&[b; 32]).unwrap()
    }
    fn public(k: &SecretKey) -> PublicKey {
        PublicKey::from_secret_key(&Secp256k1::new(), k)
    }
    const N: &str = "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141";

    /// n - s as 32 big-endian bytes.
    fn flip_s(sig: &str) -> String {
        let n = hex::decode(N).unwrap();
        let s = hex::decode(&sig[2 + 64..]).unwrap();
        let mut out = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let mut d = n[i] as i16 - s[i] as i16 - borrow;
            borrow = 0;
            if d < 0 {
                d += 256;
                borrow = 1;
            }
            out[i] = d as u8;
        }
        format!("{}{}", &sig[..66], hex::encode(out))
    }

    #[test]
    fn message_builders() {
        assert_eq!(
            challenge_message("0xab", 900, 1700000000),
            "CSR-PING-CHALLENGE-v1\n0xab\n900\n1700000000"
        );
        assert_eq!(
            binding_message("Qm1", "0x02aa", "testnet"),
            "CSR-BINDING-v1\nQm1\n0x02aa\ntestnet"
        );
        let a = Answer {
            nonce: "n".into(),
            peer_id: "p".into(),
            service_pubkey: "k".into(),
            binding_sig: "b".into(),
            network: "testnet".into(),
            csr_version: "0.1.0".into(),
            node_version: "v".into(),
            r2_state: "Synced".into(),
            tip_number: 1000,
            tip_hash: "t".into(),
            block_hash_at_height: "h".into(),
            uptime_s: 7,
            ping_mode: "inbound".into(),
        };
        assert_eq!(
            answer_message(&a),
            "CSR-PING-ANSWER-v1\nn\np\nk\nb\ntestnet\n0.1.0\nv\nSynced\n1000\nt\nh\n7\ninbound"
        );
    }

    #[test]
    fn sign_verify_roundtrip() {
        let k = key(7);
        let sig = sign(&k, "hello");
        assert_eq!(sig.len(), 130);
        assert!(verify(&public(&k), "hello", &sig));
        assert!(!verify(&public(&k), "hellp", &sig));
        assert!(!verify(&public(&key(8)), "hello", &sig));
        assert!(!verify(&public(&k), "hello", "0x12"));
        assert!(!verify(&public(&k), "hello", &sig[2..]));
    }

    #[test]
    fn high_s_is_accepted_and_ours_is_low_s() {
        let k = key(9);
        let sig = sign(&k, "msg");
        let high = flip_s(&sig);
        assert_ne!(sig, high);
        assert!(verify(&public(&k), "msg", &high));
        // our own signature is low-S: s <= n/2, i.e. the first byte of s is < 0x80 or equal to n/2 prefix
        assert!(sig[66..68] <= *"7f");
    }

    #[test]
    fn nonce_format() {
        assert!(valid_nonce(&format!("0x{}", "a".repeat(64))));
        assert!(!valid_nonce(&format!("0x{}", "A".repeat(64))));
        assert!(!valid_nonce("0xab"));
    }
}

#[cfg(test)]
#[path = "../../../acceptance/aiworker-ping-v1-1-runner/ping_v11_tests.rs"]
mod aiworker_ping_v11_tests;
