//! Encrypted per-service key/value storage.
//!
//! Format version 2. Layout: `<root>/<code_hash hex>/<HMAC-SHA256(service key, "csr-storage-name-v2"
//! ‖ key) hex>.e`, where the service key is `HMAC-SHA256(storage key, "csr-storage-service-v2" ‖
//! code_hash hex)`. Each file holds `nonce (12) ‖ ChaCha20-Poly1305(secret flag (1) ‖ key length
//! u16 LE ‖ key ‖ value)` sealed with the service key and `"v2:" ‖ code_hash hex ‖ ":" ‖ file name`
//! as associated data, so an entry cannot be moved to another service. File names are keyed
//! hashes, so they reveal nothing about keys. `migrate_v1` converts version 1 data.
//! Quotas count plaintext key + value bytes, which equal the file size minus a fixed overhead.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MAX_KEY_BYTES: usize = 256;
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
const NONCE_LEN: usize = 12;
/// nonce + tag + secret flag + key length.
const FILE_OVERHEAD: u64 = (NONCE_LEN + 16 + 1 + 2) as u64;
const RULES_FILE: &str = "rules.bin";
pub const MAX_RULES_PER_SERVICE: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum Put {
    Ok,
    QuotaExceeded,
    TooLarge,
    InvalidArgument,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Rule {
    outpoint: String,
    key: String,
}

pub struct Store {
    root: PathBuf,
    code_hash: String,
    key: [u8; 32],
    per_service_quota: u64,
    total_quota: u64,
}

fn cipher(key: &[u8; 32]) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(key.into())
}

fn seal(key: &[u8; 32], aad: &str, plain: &[u8]) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = cipher(key)
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "encryption failed".to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

fn open(key: &[u8; 32], aad: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < NONCE_LEN + 16 {
        return Err("storage entry too short".into());
    }
    cipher(key)
        .decrypt(
            Nonce::from_slice(&data[..NONCE_LEN]),
            Payload {
                msg: &data[NONCE_LEN..],
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "storage entry failed authentication".to_string())
}

/// Writes through a temp file + rename.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let r = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    r.map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac key");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

fn service_key(key: &[u8; 32], code_hash: &str) -> [u8; 32] {
    hmac(key, &[b"csr-storage-service-v2", code_hash.as_bytes()])
}

fn name_v2(skey: &[u8; 32], key: &[u8]) -> String {
    format!(
        "{}.e",
        hex::encode(hmac(skey, &[b"csr-storage-name-v2", key]))
    )
}

fn name_v1(key: &[u8; 32], k: &[u8]) -> String {
    format!(
        "{}.e",
        hex::encode(hmac(key, &[b"csr-storage-name-v1", k]))
    )
}

fn aad_v2(code_hash: &str, name: &str) -> String {
    format!("v2:{code_hash}:{name}")
}

/// Plaintext bytes (key + value, or rules) stored in one service directory.
fn dir_usage(dir: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.ends_with(".e") || n == RULES_FILE
        })
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len().saturating_sub(FILE_OVERHEAD))
        .sum()
}

fn service_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(root) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect()
}

/// Plaintext bytes stored for a service (0 when it has no data).
pub fn service_usage(root: &Path, code_hash_hex: &str) -> u64 {
    dir_usage(&root.join(code_hash_hex))
}

impl Store {
    pub fn new(
        root: &Path,
        code_hash_hex: &str,
        key: [u8; 32],
        per_service_quota_mb: u64,
        total_quota_mb: u64,
    ) -> Store {
        Store {
            root: root.to_path_buf(),
            code_hash: code_hash_hex.to_string(),
            key,
            per_service_quota: per_service_quota_mb * 1024 * 1024,
            total_quota: total_quota_mb * 1024 * 1024,
        }
    }

    fn dir(&self) -> PathBuf {
        self.root.join(&self.code_hash)
    }

    fn skey(&self) -> [u8; 32] {
        service_key(&self.key, &self.code_hash)
    }

    fn file_name(&self, key: &[u8]) -> String {
        name_v2(&self.skey(), key)
    }

    fn read_entry(&self, key: &[u8]) -> Result<Option<(bool, Vec<u8>)>, String> {
        let name = self.file_name(key);
        let data = match fs::read(self.dir().join(&name)) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read storage entry: {e}")),
        };
        let p = open(&self.skey(), &aad_v2(&self.code_hash, &name), &data)?;
        if p.len() < 3 {
            return Err("corrupt storage entry".into());
        }
        let klen = u16::from_le_bytes([p[1], p[2]]) as usize;
        if p.len() < 3 + klen {
            return Err("corrupt storage entry".into());
        }
        Ok(Some((p[0] != 0, p[3 + klen..].to_vec())))
    }

    fn write_entry(&self, key: &[u8], secret: bool, value: &[u8]) -> Result<(), String> {
        let dir = self.dir();
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let name = self.file_name(key);
        let mut p = Vec::with_capacity(3 + key.len() + value.len());
        p.push(u8::from(secret));
        p.extend_from_slice(&(key.len() as u16).to_le_bytes());
        p.extend_from_slice(key);
        p.extend_from_slice(value);
        let aad = aad_v2(&self.code_hash, &name);
        write_atomic(&dir.join(&name), &seal(&self.skey(), &aad, &p)?)
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        Ok(self.read_entry(key)?.map(|(_, v)| v))
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<Put, String> {
        if key.is_empty() {
            return Ok(Put::InvalidArgument);
        }
        if key.len() > MAX_KEY_BYTES || value.len() > MAX_VALUE_BYTES {
            return Ok(Put::TooLarge);
        }
        let existing = self.read_entry(key)?;
        let old = existing
            .as_ref()
            .map(|(_, v)| (key.len() + v.len()) as u64)
            .unwrap_or(0);
        let new = (key.len() + value.len()) as u64;
        let own = dir_usage(&self.dir());
        let total: u64 = service_dirs(&self.root).iter().map(|d| dir_usage(d)).sum();
        if own - old.min(own) + new > self.per_service_quota
            || total - old.min(total) + new > self.total_quota
        {
            return Ok(Put::QuotaExceeded);
        }
        let secret = existing.map(|(s, _)| s).unwrap_or(false);
        self.write_entry(key, secret, value)?;
        Ok(Put::Ok)
    }

    pub fn delete(&self, key: &[u8]) -> Result<bool, String> {
        let found = match fs::remove_file(self.dir().join(self.file_name(key))) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("cannot delete storage entry: {e}")),
        };
        let hk = hex::encode(key);
        let rules = self.load_rules()?;
        if rules.iter().any(|r| r.key == hk) {
            let rest: Vec<Rule> = rules.into_iter().filter(|r| r.key != hk).collect();
            self.save_rules(&rest)?;
        }
        Ok(found)
    }

    pub fn mark_secret(&self, key: &[u8]) -> Result<bool, String> {
        match self.read_entry(key)? {
            Some((_, v)) => {
                self.write_entry(key, true, &v)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Registers a rule: the entry for `key` is deleted when `outpoint` (36 bytes) is consumed.
    /// At most `MAX_RULES_PER_SERVICE` rules per service; `rules.bin` counts in the quotas.
    pub fn delete_on_consume(&self, key: &[u8], outpoint: &[u8; 36]) -> Result<Put, String> {
        let mut rules = self.load_rules()?;
        let rule = Rule {
            outpoint: hex::encode(outpoint),
            key: hex::encode(key),
        };
        if rules.contains(&rule) {
            return Ok(Put::Ok);
        }
        if rules.len() >= MAX_RULES_PER_SERVICE {
            return Ok(Put::QuotaExceeded);
        }
        let old = serde_json::to_vec(&rules).map_err(|e| e.to_string())?.len() as u64;
        rules.push(rule);
        let new = serde_json::to_vec(&rules).map_err(|e| e.to_string())?.len() as u64;
        let grow = new - old.min(new);
        let own = dir_usage(&self.dir());
        let total: u64 = service_dirs(&self.root).iter().map(|d| dir_usage(d)).sum();
        if own + grow > self.per_service_quota || total + grow > self.total_quota {
            return Ok(Put::QuotaExceeded);
        }
        self.save_rules(&rules)?;
        Ok(Put::Ok)
    }

    fn load_rules(&self) -> Result<Vec<Rule>, String> {
        load_rules(&self.dir(), &self.skey(), &self.code_hash)
    }

    fn save_rules(&self, rules: &[Rule]) -> Result<(), String> {
        save_rules(&self.dir(), &self.skey(), &self.code_hash, rules)
    }
}

fn rules_aad(code_hash: &str) -> String {
    aad_v2(code_hash, RULES_FILE)
}

fn load_rules(dir: &Path, skey: &[u8; 32], code_hash: &str) -> Result<Vec<Rule>, String> {
    match fs::read(dir.join(RULES_FILE)) {
        Ok(d) => serde_json::from_slice(&open(skey, &rules_aad(code_hash), &d)?)
            .map_err(|e| format!("corrupt rules: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("cannot read rules: {e}")),
    }
}

fn save_rules(
    dir: &Path,
    skey: &[u8; 32],
    code_hash: &str,
    rules: &[Rule],
) -> Result<(), String> {
    let path = dir.join(RULES_FILE);
    if rules.is_empty() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove rules: {e}")),
        };
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let plain = serde_json::to_vec(rules).map_err(|e| e.to_string())?;
    write_atomic(&path, &seal(skey, &rules_aad(code_hash), &plain)?)
}

fn dir_code_hash(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Every outpoint that some service has a `delete_on_consume` rule for (without duplicates).
#[allow(dead_code)]
pub fn rule_outpoints(root: &Path, key: &[u8; 32]) -> Result<Vec<[u8; 36]>, String> {
    let mut out: Vec<[u8; 36]> = Vec::new();
    for dir in service_dirs(root) {
        let ch = dir_code_hash(&dir);
        for r in load_rules(&dir, &service_key(key, &ch), &ch)? {
            let op: [u8; 36] = hex::decode(&r.outpoint)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| "corrupt rule".to_string())?;
            if !out.contains(&op) {
                out.push(op);
            }
        }
    }
    Ok(out)
}

/// Migrates version 1 storage (one key for all services, no service binding) to version 2.
/// Returns the number of migrated entries; version 2 files are left alone.
pub fn migrate_v1(root: &Path, key: &[u8; 32]) -> Result<usize, String> {
    let mut n = 0;
    for dir in service_dirs(root) {
        let ch = dir_code_hash(&dir);
        let store = Store::new(root, &ch, *key, 0, 0);
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let is_rules = name == RULES_FILE;
            if !is_rules && !name.ends_with(".e") {
                continue;
            }
            let data = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let aad = if is_rules { RULES_FILE } else { name.as_str() };
            let Ok(plain) = open(key, aad, &data) else {
                continue; // already version 2 (or foreign)
            };
            if is_rules {
                let sealed = seal(&store.skey(), &rules_aad(&ch), &plain)?;
                write_atomic(&path, &sealed)?;
                continue;
            }
            if plain.len() < 3 {
                return Err(format!("corrupt storage entry {}", path.display()));
            }
            let klen = u16::from_le_bytes([plain[1], plain[2]]) as usize;
            if plain.len() < 3 + klen {
                return Err(format!("corrupt storage entry {}", path.display()));
            }
            let k = &plain[3..3 + klen];
            if name_v1(key, k) != name {
                continue;
            }
            store.write_entry(k, plain[0] != 0, &plain[3 + klen..])?;
            fs::remove_file(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Applies every `delete_on_consume` rule of every service for a consumed outpoint. Returns the
/// number of deleted entries. Deletions are logged (time, service, outpoint) without content.
pub fn simulate_consumed(
    root: &Path,
    key: &[u8; 32],
    outpoint: &[u8; 36],
    log: &mut dyn Write,
) -> Result<usize, String> {
    apply_consumed(root, key, outpoint, log)
}

/// Applies the rules for a consumed outpoint; see `simulate_consumed`.
pub fn apply_consumed(
    root: &Path,
    key: &[u8; 32],
    outpoint: &[u8; 36],
    log: &mut dyn Write,
) -> Result<usize, String> {
    let op_hex = hex::encode(outpoint);
    let mut deleted = 0;
    for dir in service_dirs(root) {
        let code_hash = dir_code_hash(&dir);
        let skey = service_key(key, &code_hash);
        let rules = load_rules(&dir, &skey, &code_hash)?;
        if !rules.iter().any(|r| r.outpoint == op_hex) {
            continue;
        }
        let store = Store::new(root, &code_hash, *key, 0, 0);
        for r in rules.iter().filter(|r| r.outpoint == op_hex) {
            let k = hex::decode(&r.key).map_err(|_| "corrupt rule".to_string())?;
            if store.delete(&k)? {
                deleted += 1;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let _ = writeln!(
                    log,
                    "{now} delete_on_consume service=0x{code_hash} outpoint=0x{op_hex}"
                );
            }
        }
        // store.delete may have dropped rules already; reload before removing this outpoint.
        let rest: Vec<Rule> = load_rules(&dir, &skey, &code_hash)?
            .into_iter()
            .filter(|r| r.outpoint != op_hex)
            .collect();
        save_rules(&dir, &skey, &code_hash, &rest)?;
    }
    Ok(deleted)
}

#[cfg(test)]
#[path = "../../../acceptance/aiworker-csr-sandbox-storage/sandbox_storage_tests.rs"]
mod aiworker_sandbox_storage_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: &Path, per: u64) -> Store {
        Store::new(root, &"ab".repeat(32), [7; 32], per, 10)
    }

    #[test]
    fn put_get_delete_and_no_plaintext() {
        let tmp = crate::test_tmp();
        let s = store(tmp.path(), 1);
        assert_eq!(s.get(b"counter").unwrap(), None);
        assert_eq!(s.put(b"counter", b"secret-val").unwrap(), Put::Ok);
        assert_eq!(s.get(b"counter").unwrap().unwrap(), b"secret-val");
        for e in fs::read_dir(s.dir()).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            assert!(!name.contains("counter"));
            let data = fs::read(e.path()).unwrap();
            assert!(!data.windows(7).any(|w| w == b"counter"));
            assert!(!data.windows(10).any(|w| w == b"secret-val"));
        }
        assert!(s.delete(b"counter").unwrap());
        assert!(!s.delete(b"counter").unwrap());
    }

    #[test]
    fn limits_and_quota() {
        let tmp = crate::test_tmp();
        let s = store(tmp.path(), 1);
        assert_eq!(s.put(&[1; 257], b"x").unwrap(), Put::TooLarge);
        assert_eq!(
            s.put(b"k", &vec![0; MAX_VALUE_BYTES + 1]).unwrap(),
            Put::TooLarge
        );
        assert_eq!(s.put(b"", b"x").unwrap(), Put::InvalidArgument);
        assert_eq!(s.put(b"a", &vec![0; 900_000]).unwrap(), Put::Ok);
        assert_eq!(s.put(b"b", &vec![0; 200_000]).unwrap(), Put::QuotaExceeded);
        // overwriting replaces the old size
        assert_eq!(s.put(b"a", &vec![0; 1000]).unwrap(), Put::Ok);
        assert_eq!(s.put(b"b", &vec![0; 200_000]).unwrap(), Put::Ok);
    }

    #[test]
    fn tampering_is_detected() {
        let tmp = crate::test_tmp();
        let s = store(tmp.path(), 1);
        s.put(b"k", b"v").unwrap();
        let f = s.dir().join(s.file_name(b"k"));
        let mut d = fs::read(&f).unwrap();
        *d.last_mut().unwrap() ^= 1;
        fs::write(&f, d).unwrap();
        assert!(s.get(b"k").is_err());
    }

    #[test]
    fn delete_on_consume_rules() {
        let tmp = crate::test_tmp();
        let s = store(tmp.path(), 1);
        s.put(b"secret", b"v").unwrap();
        assert_eq!(s.delete_on_consume(b"secret", &[9; 36]).unwrap(), Put::Ok);
        let mut log = Vec::new();
        assert_eq!(
            simulate_consumed(tmp.path(), &[7; 32], &[1; 36], &mut log).unwrap(),
            0
        );
        assert_eq!(
            simulate_consumed(tmp.path(), &[7; 32], &[9; 36], &mut log).unwrap(),
            1
        );
        assert_eq!(s.get(b"secret").unwrap(), None);
        let l = String::from_utf8(log).unwrap();
        assert!(l.contains("delete_on_consume") && !l.contains("secret"));
    }

    #[test]
    fn mark_secret_keeps_value() {
        let tmp = crate::test_tmp();
        let s = store(tmp.path(), 1);
        assert!(!s.mark_secret(b"k").unwrap());
        s.put(b"k", b"v").unwrap();
        assert!(s.mark_secret(b"k").unwrap());
        assert_eq!(s.get(b"k").unwrap().unwrap(), b"v");
        assert_eq!(s.put(b"k", b"w").unwrap(), Put::Ok);
        assert!(s.read_entry(b"k").unwrap().unwrap().0);
    }
}
