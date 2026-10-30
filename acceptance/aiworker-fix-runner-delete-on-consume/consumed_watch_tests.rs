// Acceptance tests for applying delete_on_consume rules (F-RUN-001), written by the spec author.
// Included from runner/src/storage/mod.rs with #[cfg(test)] #[path = ...] mod aiworker_consumed_watch_tests;
use super::*;

const KEY: [u8; 32] = [7; 32];

fn op(b: u8) -> [u8; 36] {
    [b; 36]
}

#[test]
fn consumed_watch_lists_rule_outpoints_of_all_services() {
    let tmp = crate::test_tmp();
    let a = Store::new(tmp.path(), &"aa".repeat(32), KEY, 1, 10);
    let b = Store::new(tmp.path(), &"bb".repeat(32), KEY, 1, 10);
    a.put(b"x", b"1").unwrap();
    b.put(b"y", b"2").unwrap();
    a.delete_on_consume(b"x", &op(2)).unwrap();
    b.delete_on_consume(b"y", &op(1)).unwrap();
    b.delete_on_consume(b"z", &op(2)).unwrap();
    assert_eq!(rule_outpoints(tmp.path(), &KEY).unwrap(), vec![op(1), op(2)]);
}

#[test]
fn consumed_watch_deletes_only_spent_outpoints() {
    let tmp = crate::test_tmp();
    let a = Store::new(tmp.path(), &"aa".repeat(32), KEY, 1, 10);
    a.put(b"spent", b"1").unwrap();
    a.put(b"live", b"2").unwrap();
    a.put(b"plain", b"3").unwrap();
    a.delete_on_consume(b"spent", &op(1)).unwrap();
    a.delete_on_consume(b"live", &op(2)).unwrap();
    let mut asked: Vec<[u8; 36]> = Vec::new();
    let mut log = Vec::new();
    let n = apply_consumed(
        tmp.path(),
        &KEY,
        &mut |o: &[u8; 36]| {
            asked.push(*o);
            Ok::<bool, String>(*o != op(1))
        },
        &mut log,
    )
    .unwrap();
    assert_eq!(n, 1);
    assert_eq!(a.get(b"spent").unwrap(), None);
    assert_eq!(a.get(b"live").unwrap().unwrap(), b"2");
    assert_eq!(a.get(b"plain").unwrap().unwrap(), b"3");
    asked.sort();
    assert_eq!(asked, vec![op(1), op(2)]);
    assert_eq!(rule_outpoints(tmp.path(), &KEY).unwrap(), vec![op(2)]);
    let l = String::from_utf8(log).unwrap();
    assert!(l.contains("delete_on_consume") && l.contains(&hex::encode(op(1))), "{l}");
}

#[test]
fn consumed_watch_keeps_rules_when_the_node_fails() {
    let tmp = crate::test_tmp();
    let a = Store::new(tmp.path(), &"aa".repeat(32), KEY, 1, 10);
    a.put(b"k", b"v").unwrap();
    a.delete_on_consume(b"k", &op(3)).unwrap();
    let mut log = Vec::new();
    let n = apply_consumed(
        tmp.path(),
        &KEY,
        &mut |_o: &[u8; 36]| Err::<bool, String>("rpc down".into()),
        &mut log,
    )
    .unwrap();
    assert_eq!(n, 0);
    assert_eq!(a.get(b"k").unwrap().unwrap(), b"v");
    assert_eq!(rule_outpoints(tmp.path(), &KEY).unwrap(), vec![op(3)]);
}
