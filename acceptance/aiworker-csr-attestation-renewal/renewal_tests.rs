// Acceptance tests for csr-attestation-renewal, written by the spec author.
// Included from runner/src/chain.rs with #[cfg(test)] #[path = ...] mod aiworker_renewal_tests;
// Integration (Oct 2026): adapted to Registry v3 and CSR-ATTEST-v2 from contracts-v3.
use super::*;
use serde_json::json;

const PK: &str = "0x02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27";

fn att(peer: &str, pubkey: &str, expiry: u64) -> Value {
    json!({
        "peer_id": peer, "service_pubkey": pubkey, "network": "testnet",
        "expiry_ms": expiry, "signature": format!("0x{}", "22".repeat(65)),
        "stake": format!("0x{}:3", "5a".repeat(32)),
        "registry_type_hash": format!("0x{}", "4e".repeat(32)),
    })
}

fn existing() -> (Vec<u8>, OutPoint) {
    let stake = out_point_from(&format!("0x{}", "5a".repeat(32)), 3).unwrap();
    let a = Attestation::parse(&att("QmPeerA", PK, 1_000)).unwrap();
    let data = a
        .registry_v3(&H256::from_slice(&[0x77u8; 32]).unwrap(), "https://203.0.113.7:8131")
        .unwrap()
        .encode();
    (data, stake)
}

#[test]
fn refresh_replaces_only_the_attestation() {
    let (old, stake) = existing();
    let new = refresh_registry_data(&old, &att("QmPeerA", PK, 9_000)).unwrap();
    let r = parse_registry_data(&new).unwrap();
    assert_eq!(r.expiry_ms, 9_000);
    assert_eq!(r.peer_id, "QmPeerA");
    assert_eq!(r.payout_lock_hash, vec![0x77u8; 32]);
    assert_eq!(r.stake.unwrap(), stake);
    assert_eq!(new.len(), old.len(), "same layout, same capacity");
    assert_eq!(RegistryV3::decode(&new).unwrap().endpoint, "https://203.0.113.7:8131");
    let mut other_stake = att("QmPeerA", PK, 9_000);
    other_stake["stake"] = json!(format!("0x{}:3", "5b".repeat(32)));
    assert!(refresh_registry_data(&old, &other_stake).is_err(), "the stake must stay the same");
}

#[test]
fn refresh_refuses_a_different_node_or_key() {
    let (old, _) = existing();
    let e = refresh_registry_data(&old, &att("QmPeerB", PK, 9_000)).unwrap_err();
    assert!(e.contains("peer"), "{e}");
    let other = format!("0x03{}", "11".repeat(32));
    let e = refresh_registry_data(&old, &att("QmPeerA", &other, 9_000)).unwrap_err();
    assert!(e.contains("key"), "{e}");
}

#[test]
fn refresh_refuses_an_older_attestation() {
    let (old, _) = existing();
    assert!(refresh_registry_data(&old, &att("QmPeerA", PK, 1_000)).is_err());
    assert!(refresh_registry_data(&old, &att("QmPeerA", PK, 500)).is_err());
}

#[test]
fn refresh_needs_a_v2_cell_with_stake() {
    assert!(refresh_registry_data(&[1, 2, 3], &att("QmPeerA", PK, 9_000)).is_err());
}

#[test]
fn renewal_is_due_two_days_before_expiry() {
    const DAY: u64 = 86_400_000;
    let expiry = 10 * DAY;
    assert!(!renewal_due(expiry, expiry - 3 * DAY));
    assert!(renewal_due(expiry, expiry - 2 * DAY));
    assert!(renewal_due(expiry, expiry - DAY));
    assert!(renewal_due(expiry, expiry + DAY), "an expired attestation is renewed too");
    assert_eq!(RENEW_BEFORE_MS, 2 * DAY);
}
