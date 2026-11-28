// csr-071-crowdcell, phase 2: the list of approved service IDs sent at check-in. Included into
// runner/src/ping/outbound.rs; `use super::*` sees `approved_service_ids`.
use super::*;

fn id(c: char) -> String {
    format!("0x{}", c.to_string().repeat(64))
}

#[test]
fn empty_input_gives_an_empty_list() {
    assert!(approved_service_ids(vec![]).is_empty());
}

#[test]
fn ids_are_sorted_and_deduplicated() {
    let got = approved_service_ids(vec![id('c'), id('a'), id('b'), id('a')]);
    assert_eq!(got, vec![id('a'), id('b'), id('c')]);
}

#[test]
fn ids_are_lowercased() {
    let upper = format!("0x{}", "AB".repeat(32));
    let lower = format!("0x{}", "ab".repeat(32));
    assert_eq!(approved_service_ids(vec![upper, lower.clone()]), vec![lower]);
}

#[test]
fn malformed_ids_are_dropped() {
    let good = id('d');
    let got = approved_service_ids(vec![
        good.clone(),
        "0x1234".to_string(),
        "not hex".to_string(),
        format!("{}", "e".repeat(64)),         // no 0x prefix
        format!("0x{}", "g".repeat(64)),       // not hex
        format!("0x{}", "e".repeat(66)),       // too long
        String::new(),
    ]);
    assert_eq!(got, vec![good]);
}

#[test]
fn the_list_is_capped_at_64_entries_after_sorting() {
    let ids: Vec<String> = (0..100u32).rev().map(|i| format!("0x{:064x}", i)).collect();
    let got = approved_service_ids(ids);
    assert_eq!(got.len(), 64);
    assert_eq!(got[0], format!("0x{:064x}", 0));
    assert_eq!(got[63], format!("0x{:064x}", 63));
}
