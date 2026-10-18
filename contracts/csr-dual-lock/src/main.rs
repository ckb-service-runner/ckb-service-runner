#![no_std]
#![no_main]

ckb_std::entry!(program_entry);
ckb_std::default_alloc!();

/// Phase 1 placeholder: accepts every transaction.
fn program_entry() -> i8 {
    0
}
