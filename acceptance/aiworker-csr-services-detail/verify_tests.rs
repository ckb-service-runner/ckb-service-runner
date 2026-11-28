// csr-services-detail, phase 1: tests of the "Verify binary" logic (runner/src/verify.rs).
use super::*;
use ckb_hash::blake2b_256;

fn hex32(b: [u8; 32]) -> String {
    format!("0x{}", hex::encode(b))
}

// The composite service ID, written out here independently of `catalog::service_id` (D49).
fn composite_id(code: &[u8], args: &[u8], datas: &[&[u8]]) -> String {
    let mut m = b"CSR-SERVICE-v1".to_vec();
    m.extend_from_slice(&blake2b_256(code));
    m.extend_from_slice(&(args.len() as u32).to_le_bytes());
    m.extend_from_slice(args);
    m.push(datas.len() as u8);
    for d in datas {
        m.extend_from_slice(&blake2b_256(d));
    }
    hex32(blake2b_256(&m))
}

const CODE_CELL: &str = "0x1111111111111111111111111111111111111111111111111111111111111111:0";
const DATA_A: &str = "0x2222222222222222222222222222222222222222222222222222222222222222:1";
const DATA_B: &str = "0x3333333333333333333333333333333333333333333333333333333333333333:2";

fn live(b: &[u8]) -> CellRead {
    CellRead::Live(b.to_vec())
}

#[test]
fn single_binary_matches_when_the_hash_of_the_code_is_the_service_id() {
    let code = b"\x7fELF pretend binary".to_vec();
    let id = hex32(blake2b_256(&code));
    let r = verify(&id, Some(CODE_CELL), Some(&live(&code)), "", &[], &[]);
    assert_eq!(r.status, Status::Match);
    assert_eq!(r.computed_id.as_deref(), Some(id.as_str()));
    assert!(r.checks.iter().all(|c| c.ok));
    assert!(r.checks.iter().any(|c| c.what == "service ID" && c.ok));
}

#[test]
fn single_binary_mismatch_when_the_cell_holds_other_code() {
    let code = b"the published code".to_vec();
    let id = hex32(blake2b_256(&code));
    let r = verify(&id, Some(CODE_CELL), Some(&live(b"somebody else's code")), "", &[], &[]);
    assert_eq!(r.status, Status::Mismatch);
    assert_ne!(r.computed_id.as_deref(), Some(id.as_str()));
    let c = r.checks.iter().find(|c| c.what == "service ID").unwrap();
    assert!(!c.ok);
    assert!(c.detail.contains(&id), "the detail names the expected ID");
}

#[test]
fn composite_service_matches_with_args_and_data_cells() {
    let code = b"js vm".to_vec();
    let (a, b) = (b"script one".to_vec(), b"script two".to_vec());
    let args = vec![1u8, 2, 3];
    let id = composite_id(&code, &args, &[&a, &b]);
    let r = verify(&id, Some(CODE_CELL), Some(&live(&code)), "010203", &[DATA_A.into(), DATA_B.into()], &[live(&a), live(&b)]);
    assert_eq!(r.status, Status::Match);
    assert_eq!(r.checks.iter().filter(|c| c.what == "data cell").count(), 2);
}

#[test]
fn composite_service_mismatches_when_a_data_cell_was_changed_or_args_differ() {
    let code = b"js vm".to_vec();
    let (a, b) = (b"script one".to_vec(), b"script two".to_vec());
    let id = composite_id(&code, &[1, 2, 3], &[&a, &b]);
    let cells: Vec<String> = vec![DATA_A.into(), DATA_B.into()];
    let tampered = verify(&id, Some(CODE_CELL), Some(&live(&code)), "010203", &cells, &[live(&a), live(b"script TWO")]);
    assert_eq!(tampered.status, Status::Mismatch);
    let wrong_args = verify(&id, Some(CODE_CELL), Some(&live(&code)), "010204", &cells, &[live(&a), live(&b)]);
    assert_eq!(wrong_args.status, Status::Mismatch);
}

#[test]
fn a_dead_or_unreadable_code_cell_is_unavailable_not_a_mismatch() {
    let id = hex32(blake2b_256(b"x"));
    let dead = verify(&id, Some(CODE_CELL), Some(&CellRead::Dead), "", &[], &[]);
    assert_eq!(dead.status, Status::Unavailable);
    assert_eq!(dead.computed_id, None);
    assert!(dead.checks.iter().any(|c| c.what == "code cell" && !c.ok && c.detail.contains("not live")));
    let failed = verify(&id, Some(CODE_CELL), Some(&CellRead::Failed("rpc timeout".into())), "", &[], &[]);
    assert_eq!(failed.status, Status::Unavailable);
    assert!(failed.checks.iter().any(|c| c.detail.contains("rpc timeout")));
}

#[test]
fn a_dead_data_cell_is_unavailable() {
    let code = b"js vm".to_vec();
    let id = composite_id(&code, &[], &[b"d"]);
    let r = verify(&id, Some(CODE_CELL), Some(&live(&code)), "", &[DATA_A.into()], &[CellRead::Dead]);
    assert_eq!(r.status, Status::Unavailable);
    assert!(r.checks.iter().any(|c| c.what == "data cell" && !c.ok));
}

#[test]
fn a_service_without_a_recorded_code_cell_is_not_recorded() {
    let id = hex32(blake2b_256(b"x"));
    let r = verify(&id, None, None, "", &[], &[]);
    assert_eq!(r.status, Status::NotRecorded);
    assert!(r.checks.is_empty());
}

#[test]
fn bad_stored_values_are_unavailable() {
    let r = verify("0x1234", Some(CODE_CELL), Some(&live(b"x")), "", &[], &[]);
    assert_eq!(r.status, Status::Unavailable);
    let id = hex32(blake2b_256(b"x"));
    let r = verify(&id, Some(CODE_CELL), Some(&live(b"x")), "zz", &[], &[]);
    assert_eq!(r.status, Status::Unavailable);
    let r = verify(&id, Some(CODE_CELL), Some(&live(b"x")), "", &[DATA_A.into()], &[]);
    assert_eq!(r.status, Status::Unavailable);
}

#[test]
fn the_service_id_is_compared_without_regard_to_case() {
    let code = b"abc".to_vec();
    let id = hex32(blake2b_256(&code)).to_uppercase().replacen("0X", "0x", 1);
    let r = verify(&id, Some(CODE_CELL), Some(&live(&code)), "", &[], &[]);
    assert_eq!(r.status, Status::Match);
}

#[test]
fn json_shape() {
    let code = b"abc".to_vec();
    let id = hex32(blake2b_256(&code));
    let j = verify(&id, Some(CODE_CELL), Some(&live(&code)), "", &[], &[]).to_json();
    assert_eq!(j["status"], "match");
    assert_eq!(j["ok"], true);
    assert_eq!(j["service_id"], id);
    assert_eq!(j["computed_id"], id);
    assert!(j["checks"].as_array().unwrap().iter().all(|c| c["what"].is_string() && c["ok"].is_boolean() && c["detail"].is_string()));
    let j = verify(&id, None, None, "", &[], &[]).to_json();
    assert_eq!(j["status"], "not_recorded");
    assert_eq!(j["ok"], false);
    assert!(j["computed_id"].is_null());
}
