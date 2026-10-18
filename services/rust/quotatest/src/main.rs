#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

static CHUNK: [u8; 65_536] = [0xAB; 65_536];

/// Writes up to 64 values of 64 KiB; the first failing `kv_put` ends with `quota-error`.
fn program_entry() -> i8 {
    for i in 0..64u8 {
        let key = [b'q', b'0' + i / 10, b'0' + i % 10];
        if csr_svc::kv_put(&key, &CHUNK) != 0 {
            csr_svc::result_put(b"quota-error");
            return 0;
        }
    }
    csr_svc::result_put(b"no-error");
    0
}
