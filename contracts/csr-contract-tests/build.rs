//! The tests load the contract binaries from `contracts/build/` (`scripts/build-contracts.sh`). If one is
//! missing (e.g. a fresh checkout), this runs the script once, into its own cargo target directory.
use std::path::PathBuf;
use std::process::Command;

const BINARIES: [&str; 6] = [
    "release/csr-dual-lock",
    "release/csr-config-type",
    "release/csr-registry",
    "release/csr-job-type",
    "release/csr-job-lock",
    "test/csr-job-lock-variant",
];

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let build = dir.join("../build");
    for b in BINARIES {
        println!("cargo:rerun-if-changed={}", build.join(b).display());
    }
    if BINARIES.iter().all(|b| build.join(b).exists()) {
        return;
    }
    let mut cmd = Command::new("bash");
    cmd.arg(dir.join("../../scripts/build-contracts.sh")).env("CARGO_TARGET_DIR", build.join("cargo-target"));
    // the outer cargo's settings for this build script must not leak into the contract build
    for (k, _) in std::env::vars() {
        if k.starts_with("CARGO_") || k == "RUSTFLAGS" || k == "RUSTC" || k == "RUSTC_WRAPPER" || k == "TARGET" {
            if k != "CARGO_HOME" {
                cmd.env_remove(k);
            }
        }
    }
    cmd.env("CARGO_TARGET_DIR", build.join("cargo-target"));
    let status = cmd.status().expect("run scripts/build-contracts.sh");
    assert!(status.success(), "scripts/build-contracts.sh failed");
}
