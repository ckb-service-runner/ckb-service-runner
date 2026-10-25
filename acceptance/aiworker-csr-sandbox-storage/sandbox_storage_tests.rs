// Acceptance tests for csr-sandbox-storage, written by the spec author.
// Included from runner/src/storage/mod.rs with #[cfg(test)] #[path = ...] mod aiworker_sandbox_storage_tests;
use super::*;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fs;

const KEY: [u8; 32] = [7; 32];

fn svc(c: &str) -> String {
    c.repeat(32)
}

/// Writes an entry exactly as the version 1 format did (file name and AAD without the service).
fn write_v1_entry(root: &Path, code_hash: &str, key: &[u8], value: &[u8]) -> std::path::PathBuf {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&KEY).unwrap();
    mac.update(b"csr-storage-name-v1");
    mac.update(key);
    let name = format!("{}.e", hex::encode(mac.finalize().into_bytes()));
    let mut plain = vec![0u8];
    plain.extend_from_slice(&(key.len() as u16).to_le_bytes());
    plain.extend_from_slice(key);
    plain.extend_from_slice(value);
    let path = root.join(code_hash).join(&name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, seal_v1(&name, &plain)).unwrap();
    path
}

fn seal_v1(aad: &str, plain: &[u8]) -> Vec<u8> {
    let nonce = [3u8; 12];
    let ct = ChaCha20Poly1305::new((&KEY).into())
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad: aad.as_bytes() })
        .unwrap();
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    out
}

#[test]
fn entries_are_bound_to_their_service() {
    let tmp = crate::test_tmp();
    let a = Store::new(tmp.path(), &svc("aa"), KEY, 1, 10);
    let b = Store::new(tmp.path(), &svc("bb"), KEY, 1, 10);
    assert_eq!(a.put(b"token", b"value-of-a").unwrap(), Put::Ok);
    // Copy every file of service A into service B's directory under the same names.
    let bdir = tmp.path().join(svc("bb"));
    fs::create_dir_all(&bdir).unwrap();
    for e in fs::read_dir(tmp.path().join(svc("aa"))).unwrap().flatten() {
        fs::copy(e.path(), bdir.join(e.file_name())).unwrap();
    }
    let got = b.get(b"token");
    assert!(!matches!(got, Ok(Some(ref v)) if v.as_slice() == b"value-of-a"), "a copied entry must not be readable by another service");
    assert_eq!(a.get(b"token").unwrap().unwrap(), b"value-of-a");
}

#[test]
fn version_1_entries_are_migrated() {
    let tmp = crate::test_tmp();
    let old = write_v1_entry(tmp.path(), &svc("aa"), b"counter", b"41");
    assert_eq!(migrate_v1(tmp.path(), &KEY).unwrap(), 1);
    assert!(!old.exists(), "the version 1 file is removed after migration");
    let a = Store::new(tmp.path(), &svc("aa"), KEY, 1, 10);
    assert_eq!(a.get(b"counter").unwrap().unwrap(), b"41");
    assert_eq!(migrate_v1(tmp.path(), &KEY).unwrap(), 0, "a second migration finds nothing");
    assert_eq!(a.get(b"counter").unwrap().unwrap(), b"41");
}

#[test]
fn version_1_rules_are_migrated() {
    let tmp = crate::test_tmp();
    write_v1_entry(tmp.path(), &svc("aa"), b"secret", b"x");
    let op = [9u8; 36];
    let rules = format!("[{{\"outpoint\":\"{}\",\"key\":\"{}\"}}]", hex::encode(op), hex::encode(b"secret"));
    fs::write(tmp.path().join(svc("aa")).join("rules.bin"), seal_v1("rules.bin", rules.as_bytes())).unwrap();
    migrate_v1(tmp.path(), &KEY).unwrap();
    assert_eq!(rule_outpoints(tmp.path(), &KEY).unwrap(), vec![op]);
}

#[test]
fn rules_count_in_the_quota() {
    let tmp = crate::test_tmp();
    let a = Store::new(tmp.path(), &svc("aa"), KEY, 1, 10);
    a.put(b"k", b"v").unwrap();
    let before = service_usage(tmp.path(), &svc("aa"));
    for i in 0..10u8 {
        assert_eq!(a.delete_on_consume(b"k", &[i; 36]).unwrap(), Put::Ok);
    }
    assert!(service_usage(tmp.path(), &svc("aa")) > before + 300, "rules.bin must count in the service usage");
}

#[test]
fn rules_are_capped_per_service() {
    let tmp = crate::test_tmp();
    let a = Store::new(tmp.path(), &svc("aa"), KEY, 1, 10);
    a.put(b"k", b"v").unwrap();
    for i in 0..MAX_RULES_PER_SERVICE {
        let mut op = [0u8; 36];
        op[..4].copy_from_slice(&(i as u32).to_le_bytes());
        assert_eq!(a.delete_on_consume(b"k", &op).unwrap(), Put::Ok, "rule {i}");
    }
    assert_eq!(a.delete_on_consume(b"k", &[0xff; 36]).unwrap(), Put::QuotaExceeded);
}

#[test]
fn rules_of_deleted_entries_are_dropped() {
    let tmp = crate::test_tmp();
    let a = Store::new(tmp.path(), &svc("aa"), KEY, 1, 10);
    a.put(b"gone", b"1").unwrap();
    a.put(b"kept", b"2").unwrap();
    a.delete_on_consume(b"gone", &[1; 36]).unwrap();
    a.delete(b"gone").unwrap();
    a.delete_on_consume(b"kept", &[2; 36]).unwrap();
    // A rule may name a key that does not exist yet (host-interface.md, 7014).
    a.delete_on_consume(b"later", &[3; 36]).unwrap();
    assert_eq!(rule_outpoints(tmp.path(), &KEY).unwrap(), vec![[2u8; 36], [3u8; 36]]);
}

#[test]
fn host_calls_cost_by_size() {
    use crate::runtime::host::{data_cycles, MAX_DEBUG_SCAN, NODE_CALL_CYCLES};
    assert_eq!(data_cycles(0), 0);
    assert!(data_cycles(1024 * 1024) >= 262_144);
    assert!(data_cycles(4096) > data_cycles(1024));
    assert!(NODE_CALL_CYCLES >= 100_000);
    assert_eq!(MAX_DEBUG_SCAN, 1024);
}

#[test]
fn vm_memory_must_be_4_mib() {
    use crate::config::Config;
    assert!(Config::parse("[limits]\nmax_vm_memory_mb = 4").is_ok());
    let e = Config::parse("[limits]\nmax_vm_memory_mb = 8").unwrap_err();
    assert!(e.contains("max_vm_memory_mb"), "{e}");
}
