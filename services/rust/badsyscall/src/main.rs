#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Invokes syscall 9999, which no host implements.
fn program_entry() -> i8 {
    unsafe { csr_svc::syscall(9999, [0; 6]) };
    0
}
