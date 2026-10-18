//! Raw access to the CSR service calls (spec/host-interface.md).
#![no_std]

pub const RESULT_PUT: u64 = 7031;
pub const JOB_PAYLOAD: u64 = 7001;
pub const KV_GET: u64 = 7010;
pub const KV_PUT: u64 = 7011;
pub const KV_DELETE: u64 = 7012;
pub const KV_MARK_SECRET: u64 = 7013;
pub const DELETE_ON_CONSUME: u64 = 7014;
pub const SIGN: u64 = 7020;

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

/// `kv_get`: copies the value into `buf`; returns (status, full value length).
pub fn kv_get(key: &[u8], buf: &mut [u8]) -> (u64, u64) {
    let mut len = buf.len() as u64;
    let r = unsafe {
        syscall(
            KV_GET,
            [
                key.as_ptr() as u64,
                key.len() as u64,
                buf.as_mut_ptr() as u64,
                &mut len as *mut u64 as u64,
                0,
                0,
            ],
        )
    };
    (r, len)
}

/// `kv_put`; returns the host status (0 ok, 10 quota exceeded, 11 too large).
pub fn kv_put(key: &[u8], val: &[u8]) -> u64 {
    unsafe {
        syscall(
            KV_PUT,
            [key.as_ptr() as u64, key.len() as u64, val.as_ptr() as u64, val.len() as u64, 0, 0],
        )
    }
}

/// `kv_delete`; 0 ok, 1 missing.
pub fn kv_delete(key: &[u8]) -> u64 {
    unsafe { syscall(KV_DELETE, [key.as_ptr() as u64, key.len() as u64, 0, 0, 0, 0]) }
}

/// `kv_mark_secret`; 0 ok, 1 missing.
pub fn kv_mark_secret(key: &[u8]) -> u64 {
    unsafe { syscall(KV_MARK_SECRET, [key.as_ptr() as u64, key.len() as u64, 0, 0, 0, 0]) }
}

/// `delete_on_consume(key, outpoint)` with a 36-byte outpoint.
pub fn delete_on_consume(key: &[u8], outpoint: &[u8; 36]) -> u64 {
    unsafe {
        syscall(
            DELETE_ON_CONSUME,
            [key.as_ptr() as u64, key.len() as u64, outpoint.as_ptr() as u64, 0, 0, 0],
        )
    }
}

/// `sign(msg32, sig_out64)`.
pub fn sign(msg: &[u8; 32], sig: &mut [u8; 64]) -> u64 {
    unsafe { syscall(SIGN, [msg.as_ptr() as u64, sig.as_mut_ptr() as u64, 0, 0, 0, 0]) }
}
