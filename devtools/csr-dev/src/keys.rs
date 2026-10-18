//! Keys and the secp256k1_blake160 lock helpers.

use ckb_hash::blake2b_256;
use ckb_types::{
    bytes::Bytes,
    core::ScriptHashType,
    h256,
    packed::{self, Byte32, Script},
    prelude::*,
    H256,
};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde_json::Value;

pub const SIGHASH_CODE_HASH: H256 =
    h256!("0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8");

/// Public devnet keys (see `devtools/devnet-keys.json`).
const DEVNET_KEYS: &str = include_str!("../../devnet-keys.json");

pub struct Key {
    pub sk: SecretKey,
    pub pubkey: [u8; 33],
    pub args: [u8; 20],
}

impl Key {
    pub fn from_hex(s: &str) -> Result<Key, String> {
        let bytes = hex::decode(s.trim().trim_start_matches("0x"))
            .map_err(|_| "private key is not hex".to_string())?;
        let sk = SecretKey::from_slice(&bytes).map_err(|e| format!("invalid private key: {e}"))?;
        let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), &sk).serialize();
        let mut args = [0u8; 20];
        args.copy_from_slice(&blake2b_256(pubkey)[..20]);
        Ok(Key { sk, pubkey, args })
    }

    pub fn lock(&self) -> Script {
        sighash_lock(&self.args)
    }
}

pub fn sighash_lock(args: &[u8]) -> Script {
    Script::new_builder()
        .code_hash(SIGHASH_CODE_HASH.pack())
        .hash_type(packed::Byte::new(ScriptHashType::Type as u8))
        .args(Bytes::copy_from_slice(args).pack())
        .build()
}

pub fn script_hash(s: &Script) -> Byte32 {
    s.calc_script_hash()
}

/// One of the keys of `devnet-keys.json` (`genesis`, `miner`, `monitor`).
pub fn devnet_key(name: &str) -> Result<Key, String> {
    let v: Value = serde_json::from_str(DEVNET_KEYS).map_err(|e| e.to_string())?;
    let k = v[name]["privkey"]
        .as_str()
        .ok_or_else(|| format!("devnet-keys.json has no {name} key"))?;
    Key::from_hex(k)
}

pub fn h(b: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(b))
}
