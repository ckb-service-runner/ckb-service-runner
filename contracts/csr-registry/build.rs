//! Embeds the code hashes (data hashes) of the built `csr-dual-lock` and `csr-config-type` binaries, so that
//! the registry can check that its output lock is the real dual lock and find the monitor config cell by the
//! type hash of the real config type. `scripts/build-contracts.sh` builds both first.
use std::path::PathBuf;

fn code_hash(name: &str) -> [u8; 32] {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let bin = dir.join("../build/release").join(name);
    println!("cargo:rerun-if-changed={}", bin.display());
    let data = std::fs::read(&bin)
        .unwrap_or_else(|e| panic!("cannot read {} ({e}); build {name} first (scripts/build-contracts.sh)", bin.display()));
    let mut out = [0u8; 32];
    let mut h = blake2b_ref::Blake2bBuilder::new(32).personal(b"ckb-default-hash").build();
    h.update(&data);
    h.finalize(&mut out);
    out
}

fn main() {
    let path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("code_hashes.rs");
    std::fs::write(
        path,
        format!(
            "pub const DUAL_LOCK_CODE_HASH: [u8; 32] = {:?};\npub const CONFIG_TYPE_CODE_HASH: [u8; 32] = {:?};\n",
            code_hash("csr-dual-lock"),
            code_hash("csr-config-type")
        ),
    )
    .unwrap();
}
