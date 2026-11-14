#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Exercises `random_bytes` (7050). Result (68 bytes) = first 32-byte draw ‖ second 32-byte draw ‖
/// the four return codes as one byte each: two good draws (0, 0), `len = 1025` (11), `len = 0` (12).
/// The bytes go only to `result_put`, never to `debug`.
fn program_entry() -> i8 {
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    let r1 = csr_svc::random_bytes(&mut a);
    let r2 = csr_svc::random_bytes(&mut b);
    let mut big = [0u8; 1025];
    let r3 = csr_svc::random_bytes(&mut big);
    let r4 = csr_svc::random_bytes(&mut []);
    let mut out = [0u8; 68];
    out[..32].copy_from_slice(&a);
    out[32..64].copy_from_slice(&b);
    out[64] = r1 as u8;
    out[65] = r2 as u8;
    out[66] = r3 as u8;
    out[67] = r4 as u8;
    csr_svc::result_put(&out);
    0
}
