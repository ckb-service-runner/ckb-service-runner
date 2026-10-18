#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Writes one byte at 0x500000, outside the 4 MiB VM memory.
fn program_entry() -> i8 {
    unsafe { core::ptr::write_volatile(0x50_0000 as *mut u8, 1) };
    0
}
