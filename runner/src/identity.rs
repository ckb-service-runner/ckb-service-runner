//! Service and storage keys: generated on the first run per network, never printed or logged.

use rand::RngCore;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

// The secret keys are used by the ping and service code in later phases.
#[allow(dead_code)]
pub struct Identity {
    pub service_key: SecretKey,
    pub storage_key: [u8; 32],
    /// `0x` + 66 hex chars, compressed.
    pub service_pubkey: String,
}

fn read_key(path: &Path) -> Result<[u8; 32], String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let bytes = hex::decode(text.trim())
        .map_err(|_| format!("{} does not hold 64 hex characters", path.display()))?;
    bytes
        .try_into()
        .map_err(|_| format!("{} does not hold 64 hex characters", path.display()))
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
    f.write_all(format!("{}\n", hex::encode(key)).as_bytes())
        .and_then(|_| f.sync_all())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(key)
}

fn load_or_create(path: &Path, valid: impl Fn(&[u8; 32]) -> bool) -> Result<[u8; 32], String> {
    if path.exists() {
        let key = read_key(path)?;
        if !valid(&key) {
            return Err(format!("{} holds an invalid key", path.display()));
        }
        Ok(key)
    } else {
        create_key(path, valid)
    }
}

impl Identity {
    pub fn load_or_create(net_dir: &Path) -> Result<Identity, String> {
        let dir = net_dir.join("keys");
        fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot restrict {}: {e}", dir.display()))?;
        let sk = load_or_create(&dir.join("service.key"), |k| {
            SecretKey::from_slice(k).is_ok()
        })?;
        let storage_key = load_or_create(&dir.join("storage.key"), |_| true)?;
        let service_key = SecretKey::from_slice(&sk).map_err(|e| e.to_string())?;
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
