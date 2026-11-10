// Acceptance tests for csr-service-catalog (D49), written by the spec author.
// Included from runner/src/catalog.rs with #[cfg(test)] #[path = ...] mod aiworker_service_id_tests;
// Vectors computed with hashlib.blake2b(..., digest_size=32, person=b"ckb-default-hash").
use super::*;

fn h(s: &str) -> [u8; 32] {
    let v = hex::decode(s).unwrap();
    let mut a = [0u8; 32];
    a.copy_from_slice(&v);
    a
}

#[test]
fn single_binary_service_id_is_the_code_hash() {
    let code = [0x11u8; 32];
    assert_eq!(service_id(&code, &[], &[]), code);
}

#[test]
fn composite_service_id_with_args_and_data() {
    let got = service_id(&[0x11; 32], &[0xaa, 0xbb], &[[0x22; 32]]);
    assert_eq!(got, h("e8e694810480390458e45747bb05fd964cb016f2653f0b6808beb3409d88080a"));
}

#[test]
fn composite_service_id_with_data_only() {
    let got = service_id(&[0x11; 32], &[], &[[0x22; 32]]);
    assert_eq!(got, h("ac675653b016d0a1174ee5d071a464af00bf4cb61decbb33481a820afb35225d"));
}
