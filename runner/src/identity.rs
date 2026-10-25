//! Service and storage keys: generated on the first run per network, never printed or logged.

use blake2b_ref::Blake2bBuilder;
use rand::RngCore;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
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
}
