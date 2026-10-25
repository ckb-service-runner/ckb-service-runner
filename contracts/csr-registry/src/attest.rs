//! Registry v3 data and the monitor attestation `CSR-ATTEST-v2` (`spec/registry.md` §2, §3). `no_std`, no
//! allocation; shared with `csr-job-lock` and the contract tests via `#[path]` (needs a `mol` module at the
//! crate root, i.e. `csr-registry/src/mol.rs`).

use crate::mol;
use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

/// Domain separator of the attestation message (decision D21 A).
pub const ATTEST_DOMAIN_V2: &[u8] = b"CSR-ATTEST-v2";
/// `version` of the registry data.
pub const REGISTRY_VERSION: u8 = 3;
/// Longest `endpoint` (bytes).
pub const MAX_ENDPOINT: usize = 128;
/// Longest attestation lifetime beyond the monitor clock: `expiry_ms − clock_ms ≤ 8 days` (decision D24 A).
pub const MAX_LIFETIME_MS: u64 = 8 * 86_400_000;
pub const HASH_TYPE_DATA1: u8 = 2;
/// Hash type of `csr-config-type`: data2 (VM version 2; the contract's `ckb-types` code uses A instructions).
pub const HASH_TYPE_DATA2: u8 = 4;

/// Parsed `RegistryV3` data. Field slices are the raw molecule encodings (`peer_id`, `endpoint` and `prekeys`
/// include their 4-byte length prefix).
pub struct RegistryV3<'a> {
    pub peer_id: &'a [u8],
    pub service_pubkey: &'a [u8; 33],
    pub payout_lock_hash: &'a [u8; 32],
    pub network: u8,
    /// Out point of the stake deposit cell (`tx_hash (32) ‖ index (u32 LE)`).
    pub stake: &'a [u8; 36],
    pub expiry_ms: &'a [u8; 8],
    pub attestation_sig: &'a [u8; 65],
    pub endpoint: &'a [u8],
    pub prekeys: &'a [u8],
}

impl RegistryV3<'_> {
    pub fn expiry(&self) -> u64 {
        u64::from_le_bytes(*self.expiry_ms)
    }

    /// The attestation message of this entry for the registry type script hash `registry_type_hash`.
    pub fn message(&self, registry_type_hash: &[u8; 32]) -> [u8; 32] {
        message_v2(self.peer_id, self.service_pubkey, self.network, self.expiry_ms, self.stake, registry_type_hash)
    }
}

/// Whether `e` (the content of `endpoint`) is empty or `http(s)://…`, UTF-8 and at most `MAX_ENDPOINT` bytes.
pub fn endpoint_ok(e: &[u8]) -> bool {
    e.is_empty()
        || (e.len() <= MAX_ENDPOINT
            && (e.starts_with(b"https://") || e.starts_with(b"http://"))
            && core::str::from_utf8(e).is_ok())
}

/// Parses `molecule(RegistryV3)` strictly: exactly 10 fields, `version = 3`, `network` 0–2 and a valid
/// `endpoint`. `None` for anything else (v1/v2 tables included).
pub fn parse_v3(d: &[u8]) -> Option<RegistryV3<'_>> {
    let [version, peer_id, service_pubkey, payout, network, stake, expiry, sig, endpoint, prekeys] = mol::table::<10>(d)?;
    if mol::fixed::<1>(version)?[0] != REGISTRY_VERSION {
        return None;
    }
    mol::bytes(peer_id)?;
    let network = mol::fixed::<1>(network)?[0];
    if network > 2 {
        return None;
    }
    if !endpoint_ok(mol::bytes(endpoint)?) {
        return None;
    }
    mol::bytes(prekeys)?;
    Some(RegistryV3 {
        peer_id,
        service_pubkey: mol::fixed::<33>(service_pubkey)?,
        payout_lock_hash: mol::fixed::<32>(payout)?,
        network,
        stake: mol::fixed::<36>(stake)?,
        expiry_ms: mol::fixed::<8>(expiry)?,
        attestation_sig: mol::fixed::<65>(sig)?,
        endpoint,
        prekeys,
    })
}

fn hasher() -> blake2b_ref::Blake2b {
    blake2b_ref::Blake2bBuilder::new(32).personal(b"ckb-default-hash").build()
}

/// `blake2b-256("CSR-ATTEST-v2" ‖ molecule(AttestationBodyV2))`; `peer_id` is the molecule `Bytes` encoding.
pub fn message_v2(
    peer_id: &[u8],
    service_pubkey: &[u8; 33],
    network: u8,
    expiry_ms: &[u8; 8],
    stake: &[u8; 36],
    registry_type_hash: &[u8; 32],
) -> [u8; 32] {
    let network = [network];
    let fields: [&[u8]; 6] = [peer_id, service_pubkey, &network, expiry_ms, stake, registry_type_hash];
    let mut header = [0u8; 28];
    let mut off = header.len();
    for (i, f) in fields.iter().enumerate() {
        header[4 + 4 * i..8 + 4 * i].copy_from_slice(&(off as u32).to_le_bytes());
        off += f.len();
    }
    header[0..4].copy_from_slice(&(off as u32).to_le_bytes());
    let mut h = hasher();
    h.update(ATTEST_DOMAIN_V2);
    h.update(&header);
    for f in fields {
        h.update(f);
    }
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

/// Recovers the compressed public key from `r ‖ s` (64 bytes, low-S) and a recovery id 0–3.
pub fn recover_parts(msg: &[u8; 32], rs: &[u8], recid: u8) -> Option<[u8; 33]> {
    let recid = RecoveryId::from_byte(recid)?;
    let signature = Signature::from_slice(rs).ok()?;
    let vk = VerifyingKey::recover_from_prehash(msg, &signature, recid).ok()?;
    vk.to_encoded_point(true).as_bytes().try_into().ok()
}

/// Recovers the compressed public key from a 65-byte `r ‖ s ‖ recovery_id` signature (low-S required).
pub fn recover(msg: &[u8; 32], sig: &[u8; 65]) -> Option<[u8; 33]> {
    recover_parts(msg, &sig[..64], sig[64])
}

/// Hash of the molecule Script `code_hash ‖ hash_type ‖ args` with 32-byte args.
pub fn script_hash_32(code_hash: &[u8; 32], hash_type: u8, args: &[u8; 32]) -> [u8; 32] {
    // Script table: total 85, offsets 16/48/49, code_hash, hash_type, args (Bytes of 32).
    let mut s = [0u8; 85];
    s[0..4].copy_from_slice(&85u32.to_le_bytes());
    s[4..8].copy_from_slice(&16u32.to_le_bytes());
    s[8..12].copy_from_slice(&48u32.to_le_bytes());
    s[12..16].copy_from_slice(&49u32.to_le_bytes());
    s[16..48].copy_from_slice(code_hash);
    s[48] = hash_type;
    s[49..53].copy_from_slice(&32u32.to_le_bytes());
    s[53..85].copy_from_slice(args);
    let mut h = hasher();
    h.update(&s);
    let mut out = [0u8; 32];
    h.finalize(&mut out);
    out
}

/// Type hash of the monitor config cell: `csr-config-type` (code hash `config_code_hash`, hash type data2) with
/// `args = config_id`.
pub fn config_type_hash(config_code_hash: &[u8; 32], config_id: &[u8; 32]) -> [u8; 32] {
    script_hash_32(config_code_hash, HASH_TYPE_DATA2, config_id)
}

/// Config cell data: `(monitor_pubkey, clock_ms)`; `None` if shorter than 41 bytes.
pub fn config_fields(config: &[u8]) -> Option<(&[u8; 33], u64)> {
    let pk = config.get(0..33)?.try_into().ok()?;
    let clock = u64::from_le_bytes(config.get(33..41)?.try_into().ok()?);
    Some((pk, clock))
}

/// The outcome of the attestation check against the monitor config.
#[derive(Debug, PartialEq, Eq)]
pub enum AttestError {
    Signature,
    Expired,
    LifetimeOverCap,
}

/// Signature by `monitor_pubkey` over the v2 message, `expiry_ms > clock_ms` and the 8-day cap.
pub fn check_attestation(
    r: &RegistryV3,
    registry_type_hash: &[u8; 32],
    monitor_pubkey: &[u8; 33],
    clock_ms: u64,
) -> Result<(), AttestError> {
    match recover(&r.message(registry_type_hash), r.attestation_sig) {
        Some(pk) if pk == *monitor_pubkey => {}
        _ => return Err(AttestError::Signature),
    }
    let expiry = r.expiry();
    if expiry <= clock_ms {
        return Err(AttestError::Expired);
    }
    if expiry - clock_ms > MAX_LIFETIME_MS {
        return Err(AttestError::LifetimeOverCap);
    }
    Ok(())
}

/// `(owner hash, runner hash)` of `csr-dual-lock` args: 32 bytes (runner-owned, owner = runner) or 64 bytes
/// (`owner ‖ runner`); `None` for any other length.
pub fn dual_lock_hashes(args: &[u8]) -> Option<(&[u8], &[u8])> {
    match args.len() {
        32 => Some((args, args)),
        64 => Some(args.split_at(32)),
        _ => None,
    }
}
