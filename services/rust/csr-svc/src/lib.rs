//! Raw access to the CSR service calls (spec/host-interface.md).
#![no_std]

pub const RESULT_PUT: u64 = 7031;
pub const JOB_PAYLOAD: u64 = 7001;

/// Raw RISC-V `ecall` with up to six arguments.
///
/// # Safety
/// The arguments must be valid for the syscall `n` (pointers into live memory).
#[inline(always)]
pub unsafe fn syscall(n: u64, a: [u64; 6]) -> u64 {
    let mut r0 = a[0];
    #[cfg(target_arch = "riscv64")]
    core::arch::asm!(
        "ecall",
        inout("a0") r0,
        in("a1") a[1],
        in("a2") a[2],
        in("a3") a[3],
        in("a4") a[4],
        in("a5") a[5],
        in("a7") n,
    );
    #[cfg(not(target_arch = "riscv64"))]
    {
        let _ = (n, a);
        r0 = u64::MAX;
    }
    r0
}

/// `result_put(addr, len)`; returns the host status (0 = ok).
pub fn result_put(data: &[u8]) -> u64 {
    unsafe { syscall(RESULT_PUT, [data.as_ptr() as u64, data.len() as u64, 0, 0, 0, 0]) }
}

/// `job_payload(addr, len_ptr, offset)`: copies the payload into `buf`, returns (status, full length).
pub fn job_payload(buf: &mut [u8]) -> (u64, u64) {
    let mut len = buf.len() as u64;
    let r = unsafe {
        syscall(
            JOB_PAYLOAD,
            [buf.as_mut_ptr() as u64, &mut len as *mut u64 as u64, 0, 0, 0, 0],
        )
    };
    (r, len)
}
