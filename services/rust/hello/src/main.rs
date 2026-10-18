#![no_std]
#![no_main]

use alloc::vec;

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Result = `hello:` followed by the payload (`job_payload`).
fn program_entry() -> i8 {
    // `len` is the full payload length even when the buffer is smaller.
    let mut probe = [0u8; 0];
    let (r, len) = csr_svc::job_payload(&mut probe);
    if r != 0 {
        return 1;
    }
    let mut out = vec![0u8; 6 + len as usize];
    out[..6].copy_from_slice(b"hello:");
    let (r, _) = csr_svc::job_payload(&mut out[6..]);
    if r != 0 || csr_svc::result_put(&out) != 0 {
        return 2;
    }
    0
}
