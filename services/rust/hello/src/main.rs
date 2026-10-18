#![no_std]
#![no_main]

use alloc::vec::Vec;
use ckb_std::ckb_constants::Source;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Result = `hello:` followed by the payload (witness 0).
fn program_entry() -> i8 {
    let payload = match ckb_std::high_level::load_witness(0, Source::Input) {
        Ok(p) => p,
        Err(_) => return 1,
    };
    let mut out: Vec<u8> = Vec::with_capacity(6 + payload.len());
    out.extend_from_slice(b"hello:");
    out.extend_from_slice(&payload);
    if csr_svc::result_put(&out) != 0 {
        return 2;
    }
    0
}
