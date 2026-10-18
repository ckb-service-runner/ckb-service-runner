#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

static DATA: [u8; 70_000] = [0x5a; 70_000];

/// `result_put` with 70,000 bytes (the limit is 65,536).
fn program_entry() -> i8 {
    csr_svc::result_put(&DATA);
    0
}
