// Acceptance tests for contracts-v3 (runner side), written by the spec author.
// Included from runner/src/chain.rs with #[cfg(test)] #[path = ...] mod aiworker_registry_v3_tests;
use super::*;

fn v() -> serde_json::Value {
    serde_json::from_str(include_str!("registry-v3.json")).unwrap()
}

fn h(k: &str) -> Vec<u8> {
    hex::decode(v()[k].as_str().unwrap()).unwrap()
}

fn stake36() -> [u8; 36] {
    let mut s = [0x5au8; 36];
    s[32..].copy_from_slice(&3u32.to_le_bytes());
    s
}

fn sample(endpoint: &str) -> RegistryV3 {
    RegistryV3 {
        peer_id: v()["peer_id"].as_str().unwrap().to_string(),
        service_pubkey: h("service_pubkey").try_into().unwrap(),
        payout_lock_hash: [0x77; 32],
        network: 1,
        stake: stake36(),
        expiry_ms: 1_800_000_000_000,
        attestation_sig: h("attestation_signature").try_into().unwrap(),
        endpoint: endpoint.to_string(),
        prekeys: vec![],
    }
}

#[test]
fn attestation_v2_message_matches_the_vector() {
    let m = crate::attest::attestation_message_v2(
        v()["peer_id"].as_str().unwrap().as_bytes(),
        &h("service_pubkey"),
        1,
        1_800_000_000_000,
        &stake36(),
        &[0x4e; 32],
    );
    assert_eq!(hex::encode(m), v()["attestation_message_v2"].as_str().unwrap());
}

#[test]
fn registry_v3_encodes_like_the_vector() {
    assert_eq!(hex::encode(sample("").encode()), v()["registry_v3_no_endpoint"].as_str().unwrap());
    assert_eq!(
        hex::encode(sample(v()["endpoint"].as_str().unwrap()).encode()),
        v()["registry_v3_with_endpoint"].as_str().unwrap()
    );
}

#[test]
fn registry_v3_decodes() {
    let d = RegistryV3::decode(&h("registry_v3_with_endpoint")).unwrap();
    assert_eq!(d, sample(v()["endpoint"].as_str().unwrap()));
    assert!(RegistryV3::decode(&h("registry_v3_no_endpoint")[..100]).is_none());
    let mut wrong_version = h("registry_v3_no_endpoint");
    // first field (version byte) sits right after the 4 + 10 * 4 header bytes
    wrong_version[44] = 2;
    assert!(RegistryV3::decode(&wrong_version).is_none());
}

#[test]
fn registry_v3_is_smaller_than_v2() {
    assert!(sample("").encode().len() <= 300);
}

#[test]
fn generic_parse_reads_v3() {
    let r = parse_registry_data(&h("registry_v3_no_endpoint")).unwrap();
    assert_eq!(r.expiry_ms, 1_800_000_000_000);
    assert_eq!(r.payout_lock_hash, vec![0x77u8; 32]);
    assert_eq!(r.peer_id, v()["peer_id"].as_str().unwrap());
    assert!(r.stake.is_some());
}
