#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Payload must be 32 bytes; result = `sign(payload)`.
fn program_entry() -> i8 {
    let mut msg = [0u8; 32];
    let (r, len) = csr_svc::job_payload(&mut msg);
    if r != 0 || len != 32 {
        return 1;
    }
    let mut sig = [0u8; 64];
    if csr_svc::sign(&msg, &mut sig) != 0 {
        return 2;
    }
    csr_svc::result_put(&sig);
    0
}
