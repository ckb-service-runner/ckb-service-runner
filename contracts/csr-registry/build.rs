//! Embeds the code hash (data hash) of the built `csr-dual-lock` binary, so that the registry can check that
//! its output lock is the real dual lock. `scripts/build-contracts.sh` builds the dual lock first.
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let bin = dir.join("../build/release/csr-dual-lock");
    println!("cargo:rerun-if-changed={}", bin.display());
    let data = std::fs::read(&bin)
        .unwrap_or_else(|e| panic!("cannot read {} ({e}); build csr-dual-lock first (scripts/build-contracts.sh)", bin.display()));
    let mut out = [0u8; 32];
    let mut h = blake2b_ref::Blake2bBuilder::new(32).personal(b"ckb-default-hash").build();
    h.update(&data);
    h.finalize(&mut out);
    let path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("dual_lock_code_hash.rs");
    std::fs::write(path, format!("pub const DUAL_LOCK_CODE_HASH: [u8; 32] = {out:?};\n")).unwrap();
}
