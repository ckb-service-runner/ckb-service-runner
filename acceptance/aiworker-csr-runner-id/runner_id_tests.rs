// Acceptance tests for csr-runner-id, written by the spec author.
// Included from runner/src/identity.rs with #[cfg(test)] #[path = ...] mod aiworker_runner_id_tests;
use super::*;

// secp256k1 generator point (secret key 1), compressed. Vector computed with
// hashlib.blake2b(pk, digest_size=32, person=b"ckb-default-hash"), the same as csrmon.protocol.runner_id.
const G: &str = "0x0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const G_ID: &str = "0x75178f34549c5fe9cd1a0c57aebd01e7ddf9249e6fd9e01b25d6fe9108534dae";

#[test]
fn runner_id_matches_the_monitor() {
    assert_eq!(runner_id_from_pubkey_hex(G).unwrap(), G_ID);
}

#[test]
fn runner_id_accepts_upper_case() {
    assert_eq!(runner_id_from_pubkey_hex(&G.to_ascii_uppercase().replacen("0X", "0x", 1)).unwrap(), G_ID);
}

#[test]
fn runner_id_rejects_bad_keys() {
    assert!(runner_id_from_pubkey_hex("0x1234").is_err());
    assert!(runner_id_from_pubkey_hex("").is_err());
    assert!(runner_id_from_pubkey_hex(&format!("0x04{}", "11".repeat(64))).is_err());
}
