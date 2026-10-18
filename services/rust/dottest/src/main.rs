#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Mode 1 + 36-byte outpoint: store `secret` and delete it on consume. Mode 2: report presence.
fn program_entry() -> i8 {
    let mut p = [0u8; 37];
    let (r, len) = csr_svc::job_payload(&mut p);
    if r != 0 || len == 0 {
        return 1;
    }
    match p[0] {
        1 if len == 37 => {
            let mut op = [0u8; 36];
            op.copy_from_slice(&p[1..]);
            if csr_svc::kv_put(b"secret", b"secret-value") != 0
                || csr_svc::delete_on_consume(b"secret", &op) != 0
            {
                return 2;
            }
            csr_svc::result_put(b"stored");
        }
        2 => {
            let mut b = [0u8; 16];
            let (r, _) = csr_svc::kv_get(b"secret", &mut b);
            csr_svc::result_put(if r == 0 { b"present" } else { b"absent" });
        }
        _ => return 3,
    }
    0
}
