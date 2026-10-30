//! Service and storage keys: generated on the first run per network, never printed or logged.
//! The CKB node's P2P key (`node.p2p_key_file`): read only, kept in memory, used for the node-identity proof.

use blake2b_ref::Blake2bBuilder;
use rand::RngCore;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use zeroize::{Zeroize, Zeroizing};

// The secret keys are used by the ping and service code in later phases.
#[allow(dead_code)]
pub struct Identity {
    pub service_key: SecretKey,
    pub storage_key: [u8; 32],
    /// `0x` + 66 hex chars, compressed.
    pub service_pubkey: String,
}

fn read_key(path: &Path) -> Result<[u8; 32], String> {
    let text = Zeroizing::new(
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
    );
    let bad = || format!("{} does not hold 64 hex characters", path.display());
    let bytes = Zeroizing::new(hex::decode(text.trim()).map_err(|_| bad())?);
    let mut key = [0u8; 32];
    if bytes.len() != 32 {
        return Err(bad());
    }
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn create_key(path: &Path, valid: impl Fn(&[u8; 32]) -> bool) -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    loop {
        rand::rngs::OsRng.fill_bytes(&mut key);
        if valid(&key) {
            break;
        }
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    let line = Zeroizing::new(format!("{}\n", hex::encode(key)));
    f.write_all(line.as_bytes())
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(key)
}

/// True when a key file's mode gives anything to group or others.
pub fn key_mode_too_open(mode: u32) -> bool {
    mode & 0o077 != 0
}

fn load_or_create(path: &Path, valid: impl Fn(&[u8; 32]) -> bool) -> Result<[u8; 32], String> {
    if path.exists() {
        if let Ok(m) = fs::metadata(path) {
            if key_mode_too_open(m.permissions().mode()) {
                tracing::warn!("{} had mode {:o}; restricting it to 0600", path.display(), m.permissions().mode() & 0o777);
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
            }
        }
        let key = read_key(path)?;
        if !valid(&key) {
            return Err(format!("{} holds an invalid key", path.display()));
        }
        Ok(key)
    } else {
        create_key(path, valid)
    }
}

/// Code hash of `secp256k1_blake160_sighash_all` (hash type `type`) on every CKB chain.
const SIGHASH_CODE_HASH: [u8; 32] = [
    0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9,
    0xfc, 0xc8, 0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8,
];

fn ckb_hash(data: &[u8]) -> [u8; 32] {
    let mut h = Blake2bBuilder::new(32)
        .personal(b"ckb-default-hash")
        .build();
    h.update(data);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

impl Drop for Identity {
    fn drop(&mut self) {
        self.storage_key.zeroize();
    }
}

impl Identity {
    /// `(runner_lock_args, runner_lock_hash)`: the sighash lock of `blake160(service_pubkey)`, hex with `0x`.
    pub fn runner_lock(&self) -> Result<(String, String), String> {
        let pk =
            hex::decode(self.service_pubkey.trim_start_matches("0x")).map_err(|e| e.to_string())?;
        let args = ckb_hash(&pk)[..20].to_vec();
        // Molecule `Script`: total size, 3 offsets, code_hash, hash_type (type = 1), args as Bytes.
        let mut script = Vec::with_capacity(73);
        script.extend_from_slice(&73u32.to_le_bytes());
        for off in [16u32, 48, 49] {
            script.extend_from_slice(&off.to_le_bytes());
        }
        script.extend_from_slice(&SIGHASH_CODE_HASH);
        script.push(1);
        script.extend_from_slice(&20u32.to_le_bytes());
        script.extend_from_slice(&args);
        Ok((
            format!("0x{}", hex::encode(&args)),
            format!("0x{}", hex::encode(ckb_hash(&script))),
        ))
    }

    pub fn load_or_create(net_dir: &Path) -> Result<Identity, String> {
        let dir = net_dir.join("keys");
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot restrict {}: {e}", dir.display()))?;
        let mut sk = load_or_create(&dir.join("service.key"), |k| {
            SecretKey::from_slice(k).is_ok()
        })?;
        let storage_key = load_or_create(&dir.join("storage.key"), |_| true)?;
        let service_key = SecretKey::from_slice(&sk).map_err(|e| e.to_string());
        sk.zeroize();
        let service_key = service_key?;
        let public = PublicKey::from_secret_key(&Secp256k1::new(), &service_key);
        Ok(Identity {
            service_key,
            storage_key,
            service_pubkey: format!("0x{}", hex::encode(public.serialize())),
        })
    }
}

/// The CKB node's P2P secret key. Never logged, printed or serialised; erased on drop.
pub struct P2pKey {
    secret: SecretKey,
}

impl Drop for P2pKey {
    fn drop(&mut self) {
        self.secret.non_secure_erase();
    }
}

impl std::fmt::Debug for P2pKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P2pKey({})", self.peer_id())
    }
}

impl P2pKey {
    /// Compressed (33 bytes).
    pub fn public_key_bytes(&self) -> [u8; 33] {
        PublicKey::from_secret_key(&Secp256k1::new(), &self.secret).serialize()
    }

    pub fn peer_id(&self) -> String {
        peer_id_from_pubkey(&self.public_key_bytes())
    }
}

const BASE58_ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn base58_encode(data: &[u8]) -> String {
    let zeros = data.iter().take_while(|b| **b == 0).count();
    // Base-58 digits, least significant first.
    let mut digits: Vec<u8> = Vec::with_capacity(data.len() * 138 / 100 + 1);
    for &b in &data[zeros..] {
        let mut carry = b as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat('1').take(zeros));
    out.extend(digits.iter().rev().map(|d| BASE58_ALPHABET[*d as usize] as char));
    out
}

fn base58_decode(s: &str) -> Option<Vec<u8>> {
    let zeros = s.bytes().take_while(|b| *b == b'1').count();
    // Bytes, least significant first.
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    for c in s.bytes().skip(zeros) {
        let mut carry = BASE58_ALPHABET.iter().position(|a| *a == c)? as u32;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let mut out = vec![0u8; zeros];
    out.extend(bytes.iter().rev());
    Some(out)
}

/// CKB peer ID: `base58(0x12 ‖ 0x20 ‖ SHA-256(compressed public key))` (tentacle `PeerId::from_public_key`).
pub fn peer_id_from_pubkey(pk: &[u8; 33]) -> String {
    let mut inner = Vec::with_capacity(34);
    inner.extend_from_slice(&[0x12, 0x20]);
    inner.extend_from_slice(&Sha256::digest(pk));
    base58_encode(&inner)
}

/// True for a well-formed CKB peer ID (a base58 SHA-256 multihash).
pub fn valid_peer_id(s: &str) -> bool {
    base58_decode(s).is_some_and(|b| b.len() == 34 && b[0] == 0x12 && b[1] == 0x20)
}

/// 32 raw bytes, or 64 hex characters (optional 0x, surrounding whitespace); a valid secp256k1 secret.
pub fn parse_p2p_key(bytes: &[u8]) -> Result<P2pKey, String> {
    let mut raw = [0u8; 32];
    if bytes.len() == 32 {
        raw.copy_from_slice(bytes);
    } else {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "the P2P key is neither 32 raw bytes nor 64 hex characters".to_string())?;
        let t = text.trim();
        let t = t.strip_prefix("0x").unwrap_or(t);
        if t.len() != 64 || hex::decode_to_slice(t, &mut raw).is_err() {
            raw.fill(0);
            return Err("the P2P key is neither 32 raw bytes nor 64 hex characters".into());
        }
    }
    let secret = SecretKey::from_slice(&raw);
    raw.fill(0);
    std::hint::black_box(&raw);
    secret
        .map(|secret| P2pKey { secret })
        .map_err(|_| "the P2P key is not a valid secp256k1 secret key".into())
}

/// Why the P2P key file could not be used.
#[derive(Debug, Clone, PartialEq)]
pub enum P2pKeyError {
    /// Missing, not readable by this user, readable by "other", or not a key.
    Unreadable(String),
}

impl std::fmt::Display for P2pKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            P2pKeyError::Unreadable(m) => f.write_str(m),
        }
    }
}

/// Reads `node.p2p_key_file`: no write, no chmod, no change of owner. Refuses a file readable by "other".
pub fn load_p2p_key_file(path: &Path) -> Result<P2pKey, P2pKeyError> {
    let err = |m: String| P2pKeyError::Unreadable(m);
    let meta = fs::metadata(path).map_err(|e| err(format!("cannot read {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(err(format!("{} is not a file", path.display())));
    }
    if meta.permissions().mode() & 0o004 != 0 {
        return Err(err(format!(
            "{} is readable by every user (mode {:o}): remove the read bit for \"other\" on the node \
             (`chmod o-r`) and grant the csr user access with an ACL or a group (see docs/install.md)",
            path.display(),
            meta.permissions().mode() & 0o777
        )));
    }
    let mut bytes = fs::read(path).map_err(|e| err(format!("cannot read {}: {e}", path.display())))?;
    let key = parse_p2p_key(&bytes).map_err(|e| err(format!("{}: {e}", path.display())));
    bytes.fill(0);
    std::hint::black_box(&bytes);
    key
}

/// The P2P key as configured: off (`node.p2p_key_file` empty), unreadable, or loaded (in memory only).
pub enum ConfiguredP2pKey {
    Off,
    Unreadable(String),
    Loaded(P2pKey),
}

impl ConfiguredP2pKey {
    pub fn load(p2p_key_file: &str) -> ConfiguredP2pKey {
        if p2p_key_file.trim().is_empty() {
            return ConfiguredP2pKey::Off;
        }
        match load_p2p_key_file(Path::new(p2p_key_file.trim())) {
            Ok(k) => ConfiguredP2pKey::Loaded(k),
            Err(e) => ConfiguredP2pKey::Unreadable(e.to_string()),
        }
    }

    /// The identity state against the node ID the node reports (`None` = not known yet).
    pub fn status(&self, node_id: Option<&str>) -> IdentityStatus {
        let (state, message) = match (self, node_id) {
            (ConfiguredP2pKey::Off, _) => ("off", None),
            (ConfiguredP2pKey::Unreadable(e), _) => ("unreadable", Some(e.clone())),
            (ConfiguredP2pKey::Loaded(_), None) => {
                ("unknown", Some("the node does not report its node ID yet".to_string()))
            }
            (ConfiguredP2pKey::Loaded(k), Some(id)) if k.peer_id() == id => ("proven", None),
            (ConfiguredP2pKey::Loaded(k), Some(id)) => (
                "mismatch",
                Some(format!(
                    "node.p2p_key_file derives peer ID {}, but the node reports {id}: \
                     wrong key file (another node?); no identity proof is sent",
                    k.peer_id()
                )),
            ),
        };
        IdentityStatus { state, message }
    }

    /// The key, only when it derives the node ID the node reports.
    pub fn proven(&self, node_id: Option<&str>) -> Option<&P2pKey> {
        match self {
            ConfiguredP2pKey::Loaded(k) if node_id == Some(k.peer_id().as_str()) => Some(k),
            _ => None,
        }
    }
}

/// `state`: `off`, `proven`, `mismatch`, `unreadable`, or `unknown` (key loaded, node ID not known yet).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct IdentityStatus {
    pub state: &'static str,
    pub message: Option<String>,
}

pub fn identity_message(peer_id: &str, service_pubkey: &str, network: &str, ts: i64) -> String {
    format!("CSR-NODE-IDENTITY-v1\n{peer_id}\n{service_pubkey}\n{network}\n{ts}")
}

/// {"peer_id", "service_pubkey", "network", "ts", "p2p_pubkey": "0x…", "p2p_sig", "service_sig"}
pub fn identity_proof(
    p2p: &P2pKey,
    service: &SecretKey,
    service_pubkey: &str,
    network: &str,
    ts: i64,
) -> serde_json::Value {
    let peer_id = p2p.peer_id();
    let msg = identity_message(&peer_id, service_pubkey, network, ts);
    serde_json::json!({
        "peer_id": peer_id,
        "service_pubkey": service_pubkey,
        "network": network,
        "ts": ts,
        "p2p_pubkey": format!("0x{}", hex::encode(p2p.public_key_bytes())),
        "p2p_sig": crate::ping::protocol::sign(&p2p.secret, &msg),
        "service_sig": crate::ping::protocol::sign(service, &msg),
    })
}

/// "/ip4/…/tcp/8115/p2p/Qm…" (or /ip6, /dns4, /dns6) → (peer_id, address without /p2p/…)
pub fn parse_monitor_p2p(multiaddr: &str) -> Option<(String, String)> {
    let (addr, peer_id) = multiaddr.trim().rsplit_once("/p2p/")?;
    let parts: Vec<&str> = addr.split('/').collect();
    // "", proto, host, "tcp", port
    let [empty, proto, host, tcp, port] = parts[..] else {
        return None;
    };
    let ok = empty.is_empty()
        && match proto {
            "ip4" => host.parse::<std::net::Ipv4Addr>().is_ok(),
            "ip6" => host.parse::<std::net::Ipv6Addr>().is_ok(),
            "dns4" | "dns6" => !host.is_empty(),
            _ => false,
        }
        && tcp == "tcp"
        && port.parse::<u16>().is_ok_and(|p| p > 0)
        && valid_peer_id(peer_id);
    ok.then(|| (peer_id.to_string(), addr.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_lock_of_genesis_key_matches_known_args() {
        // The well-known dev key 0xd00c06…: pubkey -> args 0xc8328aab… (secp256k1_blake160).
        let sk = SecretKey::from_slice(
            &hex::decode("d00c06bfd800d27397002dca6fb0993d5ba6399b4238b2f29ee9deb97593d2bc")
                .unwrap(),
        )
        .unwrap();
        let public = PublicKey::from_secret_key(&Secp256k1::new(), &sk);
        let id = Identity {
            service_key: sk,
            storage_key: [0; 32],
            service_pubkey: format!("0x{}", hex::encode(public.serialize())),
        };
        let (args, hash) = id.runner_lock().unwrap();
        assert_eq!(args, "0xc8328aabcd9b9e8e64fbc566c4385c3bdeb219d7");
        assert_eq!(hash.len(), 66);
    }

    #[test]
    fn creates_once_with_modes_and_reloads() {
        let tmp = crate::test_tmp();
        let a = Identity::load_or_create(tmp.path()).unwrap();
        let b = Identity::load_or_create(tmp.path()).unwrap();
        assert_eq!(a.service_pubkey, b.service_pubkey);
        assert_eq!(a.service_pubkey.len(), 68);
        let f = tmp.path().join("keys/service.key");
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(tmp.path().join("keys"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let text = fs::read_to_string(f).unwrap();
        assert_eq!(text.len(), 65);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn base58_roundtrip_and_peer_id_check() {
        for data in [&[][..], &[0, 0, 1][..], &[0x12, 0x20, 0xff, 0x00, 0x7f][..]] {
            assert_eq!(base58_decode(&base58_encode(data)).unwrap(), data);
        }
        assert_eq!(base58_encode(&[0, 0, 0x01, 0x00]), "115R");
        let k = parse_p2p_key(&[7u8; 32]).unwrap();
        assert!(valid_peer_id(&k.peer_id()));
        assert!(!valid_peer_id("Qm0OIl"));
        assert!(!valid_peer_id("QmS7sX"));
        assert!(!format!("{k:?}").contains("0707"));
    }

    #[test]
    fn p2p_key_file_modes_and_read_only() {
        let tmp = crate::test_tmp();
        let f = tmp.path().join("secret_key");
        fs::write(&f, [7u8; 32]).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o640)).unwrap();
        let before = fs::metadata(&f).unwrap().modified().unwrap();
        let k = load_p2p_key_file(&f).unwrap();
        assert_eq!(k.peer_id(), "QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS");
        let m = fs::metadata(&f).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o640);
        assert_eq!(m.modified().unwrap(), before);
        fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
        let e = load_p2p_key_file(&f).unwrap_err().to_string();
        assert!(e.contains("readable by every user"), "{e}");
        assert_eq!(fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o644);
        assert!(load_p2p_key_file(&tmp.path().join("missing")).is_err());
        fs::write(&f, "0x0707").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        let e = load_p2p_key_file(&f).unwrap_err().to_string();
        assert!(!e.contains("0707"), "{e}");
    }

    #[test]
    fn configured_key_states() {
        let id = "QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS";
        assert_eq!(ConfiguredP2pKey::load("").status(Some(id)).state, "off");
        let tmp = crate::test_tmp();
        let f = tmp.path().join("secret_key");
        fs::write(&f, format!("{}\n", hex::encode([7u8; 32]))).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        let k = ConfiguredP2pKey::load(f.to_str().unwrap());
        assert_eq!(k.status(Some(id)).state, "proven");
        assert!(k.proven(Some(id)).is_some());
        assert_eq!(k.status(None).state, "unknown");
        let m = k.status(Some("QmOther"));
        assert_eq!(m.state, "mismatch");
        assert!(m.message.unwrap().contains("QmOther"));
        assert!(k.proven(Some("QmOther")).is_none());
        let u = ConfiguredP2pKey::load(tmp.path().join("none").to_str().unwrap());
        assert_eq!(u.status(Some(id)).state, "unreadable");
        assert!(u.proven(Some(id)).is_none());
    }

    #[test]
    fn monitor_p2p_rejects_bad_hosts() {
        let id = "QmS7sXpV62pY1jHiAkiVkftGxJvimJZTLdLuzerj7yafdS";
        assert!(parse_monitor_p2p(&format!("/ip6/2001:db8::1/tcp/8115/p2p/{id}")).is_some());
        assert!(parse_monitor_p2p(&format!("/ip4/999.1.1.1/tcp/8115/p2p/{id}")).is_none());
        assert!(parse_monitor_p2p(&format!("/ip4/1.2.3.4/udp/8115/p2p/{id}")).is_none());
        assert!(parse_monitor_p2p("/ip4/1.2.3.4/tcp/8115/p2p/Qmshort").is_none());
    }
}

#[cfg(test)]
#[path = "../../acceptance/aiworker-node-identity-runner/node_identity_tests.rs"]
mod aiworker_node_identity_tests;
