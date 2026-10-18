#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Infinite loop without syscalls.
fn program_entry() -> i8 {
    let mut x: u64 = 0;
    loop {
        x = core::hint::black_box(x.wrapping_add(1));
    }
}
