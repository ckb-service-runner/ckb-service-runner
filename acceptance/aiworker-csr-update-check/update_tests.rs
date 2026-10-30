// Acceptance tests for csr-update-check, written by the spec author.
// Included from runner/src/update.rs with #[cfg(test)] #[path = ...] mod aiworker_update_tests;
use super::*;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

#[test]
fn version_comparison() {
    assert_eq!(newer("0.8.2", "v0.9.0"), Some(true));
    assert_eq!(newer("0.8.2", "v0.8.10"), Some(true));
    assert_eq!(newer("0.9.0", "v0.9.0"), Some(false));
    assert_eq!(newer("1.0.0", "v0.9.9"), Some(false));
    assert_eq!(newer("0.8.2", "0.9.0"), Some(true), "the v is optional");
    assert_eq!(newer("0.8.2", "v1.0.0-rc1"), None, "pre-releases are not offered");
    assert_eq!(newer("0.8.2", "latest"), None);
}

#[test]
fn checksum_file_lookup() {
    let h = "ab".repeat(32);
    let text = format!("{}  csr-0.9.0-x86_64-linux\n{h}  csr-0.9.0-aarch64-linux\n", "cd".repeat(32));
    assert_eq!(sha256_for(&text, "csr-0.9.0-aarch64-linux"), Some([0xab; 32]));
    assert_eq!(sha256_for(&text, "csr-0.9.0-riscv64-linux"), None);
    assert_eq!(sha256_for("zz  csr\n", "csr"), None);
}

#[test]
fn asset_names() {
    assert_eq!(asset_name("v0.9.0", "x86_64"), "csr-0.9.0-x86_64-linux");
    assert_eq!(asset_name("0.9.0", "aarch64"), "csr-0.9.0-aarch64-linux");
}

#[test]
fn release_signature_is_checked() {
    let key = SigningKey::from_bytes(&[42u8; 32]);
    let pk = key.verifying_key().to_bytes();
    let sums = b"0000  csr-0.9.0-x86_64-linux\n";
    let sig = key.sign(sums).to_bytes();
    assert!(verify_release(sums, &sig, &pk));
    assert!(!verify_release(b"1111  csr-0.9.0-x86_64-linux\n", &sig, &pk));
    let other = SigningKey::from_bytes(&[43u8; 32]).verifying_key().to_bytes();
    assert!(!verify_release(sums, &sig, &other));
}

#[test]
fn staged_binary_is_checked_end_to_end() {
    let key = SigningKey::from_bytes(&[42u8; 32]);
    let pk = key.verifying_key().to_bytes();
    let binary = b"\x7fELF fake binary";
    let digest: [u8; 32] = Sha256::digest(binary).into();
    let name = "csr-0.9.0-x86_64-linux";
    let sums = format!("{}  {name}\n", hex::encode(digest));
    let sig = key.sign(sums.as_bytes()).to_bytes();
    assert!(check_staged(binary, name, sums.as_bytes(), &sig, &pk).is_ok());
    assert!(check_staged(b"tampered", name, sums.as_bytes(), &sig, &pk).is_err());
    assert!(check_staged(binary, "csr-0.9.0-aarch64-linux", sums.as_bytes(), &sig, &pk).is_err());
    let mut bad_sig = sig;
    bad_sig[0] ^= 1;
    assert!(check_staged(binary, name, sums.as_bytes(), &bad_sig, &pk).is_err());
}

#[test]
fn downloads_stay_on_github() {
    assert!(download_host_allowed("https://github.com/ckb-service-runner/ckb-service-runner/releases/download/v0.9.0/x"));
    assert!(download_host_allowed("https://objects.githubusercontent.com/github-production-release-asset/1"));
    assert!(download_host_allowed("https://api.github.com/repos/ckb-service-runner/ckb-service-runner/releases/latest"));
    assert!(!download_host_allowed("http://github.com/x"));
    assert!(!download_host_allowed("https://github.com.evil.example/x"));
    assert!(!download_host_allowed("https://example.com/x"));
}

#[test]
fn release_key_is_pinned() {
    assert_eq!(RELEASE_PUBKEY.len(), 32);
}
