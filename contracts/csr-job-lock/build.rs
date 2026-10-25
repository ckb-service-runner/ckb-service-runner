//! Embeds the code hash (data hash) of the built `csr-config-type` binary, so that the job lock finds the
//! monitor config cell by the type hash of the real config type. `scripts/build-contracts.sh` builds it first.
//! `CSR_JOB_LOCK_VARIANT` (tests only) changes an embedded constant to make a byte-different build.
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let bin = dir.join("../build/release/csr-config-type");
    println!("cargo:rerun-if-changed={}", bin.display());
    println!("cargo:rerun-if-env-changed=CSR_JOB_LOCK_VARIANT");
    let data = std::fs::read(&bin)
        .unwrap_or_else(|e| panic!("cannot read {} ({e}); build csr-config-type first (scripts/build-contracts.sh)", bin.display()));
    let mut out = [0u8; 32];
    let mut h = blake2b_ref::Blake2bBuilder::new(32).personal(b"ckb-default-hash").build();
    h.update(&data);
    h.finalize(&mut out);
    let variant = std::env::var("CSR_JOB_LOCK_VARIANT").unwrap_or_default();
    let path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("code_hashes.rs");
    std::fs::write(
        path,
        format!("pub const CONFIG_TYPE_CODE_HASH: [u8; 32] = {out:?};\npub const BUILD_VARIANT: &str = {variant:?};\n"),
    )
    .unwrap();
}
