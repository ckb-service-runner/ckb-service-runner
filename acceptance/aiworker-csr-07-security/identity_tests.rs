// Acceptance tests for csr-07-security phase 1 (S6), written by the spec author.
// Included from runner/src/identity.rs.
use super::*;
use std::fs;
use std::path::{Path, PathBuf};

const SK1: &str = "0000000000000000000000000000000000000000000000000000000000000001";
const SK2: &str = "0000000000000000000000000000000000000000000000000000000000000002";
// Compressed public key of secret key 1 (the secp256k1 generator point).
const PK1_HEX: &str = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

fn write_key(dir: &Path, hex_key: &str) -> PathBuf {
    let f = dir.join("secret_key");
    fs::write(&f, hex_key).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
    }
    f
}

fn peer_id_of(hex_key: &str) -> String {
    parse_p2p_key(hex_key.as_bytes()).unwrap().peer_id()
}

fn service_key() -> SecretKey {
    SecretKey::from_slice(&[7u8; 32]).unwrap()
}

#[test]
fn generator_vector_sanity() {
    let k = parse_p2p_key(SK1.as_bytes()).unwrap();
    assert_eq!(hex::encode(k.public_key_bytes()), PK1_HEX);
}

#[test]
fn configured_key_reports_proven_without_keeping_the_key() {
    let tmp = crate::test_tmp();
    let f = write_key(tmp.path(), SK1);
    let id = peer_id_of(SK1);
    let cfg = ConfiguredP2pKey::load(f.to_str().unwrap());
    assert_eq!(cfg.status(Some(&id)).state, "proven");
    // The status needs no file: delete it and ask again.
    fs::remove_file(&f).unwrap();
    assert_eq!(cfg.status(Some(&id)).state, "proven");
    assert_eq!(cfg.status(None).state, "unknown");
    assert_eq!(cfg.status(Some("QmSomeOtherNode")).state, "mismatch");
}

#[test]
fn proof_is_signed_from_the_file_at_signing_time() {
    let tmp = crate::test_tmp();
    let f = write_key(tmp.path(), SK1);
    let id = peer_id_of(SK1);
    let cfg = ConfiguredP2pKey::load(f.to_str().unwrap());
    let proof = cfg
        .sign_identity_proof(Some(&id), &service_key(), "0x02aa", "testnet", 1_700_000_000)
        .expect("the proof is signed while the key file exists");
    assert_eq!(proof["peer_id"].as_str(), Some(id.as_str()));
    assert_eq!(proof["p2p_pubkey"].as_str(), Some(format!("0x{PK1_HEX}").as_str()));
    assert_eq!(proof["network"].as_str(), Some("testnet"));
    assert_eq!(proof["service_pubkey"].as_str(), Some("0x02aa"));
    assert_eq!(proof["ts"].as_i64(), Some(1_700_000_000));
    assert!(proof["p2p_sig"].as_str().is_some_and(|s| s.len() > 10));
    assert!(proof["service_sig"].as_str().is_some_and(|s| s.len() > 10));
    // The secret never appears in the proof.
    assert!(!proof.to_string().contains(SK1));
}

#[test]
fn signing_fails_when_the_file_is_gone() {
    let tmp = crate::test_tmp();
    let f = write_key(tmp.path(), SK1);
    let id = peer_id_of(SK1);
    let cfg = ConfiguredP2pKey::load(f.to_str().unwrap());
    fs::remove_file(&f).unwrap();
    let err = cfg
        .sign_identity_proof(Some(&id), &service_key(), "0x02aa", "testnet", 1)
        .unwrap_err();
    assert!(!err.contains(SK1));
}

#[test]
fn signing_fails_when_the_file_now_holds_another_key() {
    let tmp = crate::test_tmp();
    let f = write_key(tmp.path(), SK1);
    let id = peer_id_of(SK1);
    let cfg = ConfiguredP2pKey::load(f.to_str().unwrap());
    fs::write(&f, SK2).unwrap();
    let err = cfg
        .sign_identity_proof(Some(&id), &service_key(), "0x02aa", "testnet", 1)
        .unwrap_err();
    assert!(!err.contains(SK1) && !err.contains(SK2));
}

#[test]
fn signing_fails_for_a_different_or_missing_node_id() {
    let tmp = crate::test_tmp();
    let f = write_key(tmp.path(), SK1);
    let cfg = ConfiguredP2pKey::load(f.to_str().unwrap());
    assert!(cfg
        .sign_identity_proof(Some(&peer_id_of(SK2)), &service_key(), "0x02aa", "testnet", 1)
        .is_err());
    assert!(cfg
        .sign_identity_proof(None, &service_key(), "0x02aa", "testnet", 1)
        .is_err());
}

#[test]
fn off_and_unreadable_never_sign() {
    let off = ConfiguredP2pKey::load("   ");
    assert_eq!(off.status(Some("Qm1")).state, "off");
    assert!(off
        .sign_identity_proof(Some("Qm1"), &service_key(), "0x02aa", "testnet", 1)
        .is_err());
    let tmp = crate::test_tmp();
    let missing = tmp.path().join("no_such_key");
    let bad = ConfiguredP2pKey::load(missing.to_str().unwrap());
    assert_eq!(bad.status(Some("Qm1")).state, "unreadable");
    assert!(bad
        .sign_identity_proof(Some("Qm1"), &service_key(), "0x02aa", "testnet", 1)
        .is_err());
}
