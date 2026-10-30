// Acceptance tests for csr-cell-consumed-trigger, written by the spec author.
// Included from runner/src/jobs.rs with #[cfg(test)] #[path = ...] mod aiworker_watch_tests;
use super::*;
use std::collections::HashSet;

fn op(b: u8) -> [u8; 36] {
    let mut o = [b; 36];
    o[32..].copy_from_slice(&7u32.to_le_bytes());
    o
}

#[test]
fn watch_header_roundtrip() {
    let p = watch_payload(&op(0xab), b"service data");
    assert_eq!(&p[..4], WATCH_MAGIC);
    assert_eq!(p[4], 1);
    assert_eq!(p.len(), WATCH_HEADER_LEN + 12);
    assert_eq!(watch_outpoint(&p), Some(op(0xab)));
    assert_eq!(&p[WATCH_HEADER_LEN..], b"service data");
}

#[test]
fn plain_payloads_have_no_watch() {
    assert_eq!(watch_outpoint(b""), None);
    assert_eq!(watch_outpoint(b"hello world, this is a normal payload of some length"), None);
    // Magic with an unknown version or too short is not a watch.
    let mut p = watch_payload(&op(1), b"");
    p[4] = 2;
    assert_eq!(watch_outpoint(&p), None);
    assert_eq!(watch_outpoint(&watch_payload(&op(1), b"")[..WATCH_HEADER_LEN - 1]), None);
}

#[test]
fn watched_job_waits_for_the_cell() {
    let p = watch_payload(&op(3), b"x");
    let mut dead = HashSet::new();
    assert!(!watch_due(&p, &dead));
    dead.insert(op(9));
    assert!(!watch_due(&p, &dead));
    dead.insert(op(3));
    assert!(watch_due(&p, &dead));
    assert!(watch_due(b"plain", &HashSet::new()), "a job without a watch is due as before");
}

#[test]
fn watch_label_names_the_cell() {
    let l = watch_label(&op(0xab));
    assert!(l.starts_with("on-cell-consumed:0x"), "{l}");
    assert!(l.ends_with(":7"), "{l}");
}
