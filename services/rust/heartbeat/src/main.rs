#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Demo keeper service: increments `counter` (u32 LE, missing = 0) in its storage and
/// returns the new value as the result. The transaction skeleton stays empty.
fn program_entry() -> i8 {
    let mut buf = [0u8; 4];
    let (r, len) = csr_svc::kv_get(b"counter", &mut buf);
    let old = match r {
        0 if len == 4 => u32::from_le_bytes(buf),
        1 => 0,
        _ => return 1,
    };
    let new = old.wrapping_add(1).to_le_bytes();
    if csr_svc::kv_put(b"counter", &new) != 0 {
        return 2;
    }
    csr_svc::result_put(&new);
    0
}
