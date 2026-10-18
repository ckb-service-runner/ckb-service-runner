//! CKB-VM sandbox: one VM per run, with cycle, memory and wall-time limits.

pub mod host;
pub mod manifest;
pub mod vtx;

use ckb_vm::cost_model::estimate_cycles;
use ckb_vm::machine::asm::{AsmCoreMachine, AsmMachine};
use ckb_vm::machine::{Pause, VERSION2};
use ckb_vm::{
    Bytes, DefaultMachineBuilder, DefaultMachineRunner, Error, SupportMachine, ISA_B, ISA_IMC,
    ISA_MOP,
};
use host::{Host, HostState};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// CKB-VM's fixed memory size; the pinned ckb-vm (0.24.15) cannot change it.
pub const VM_MEMORY_MB: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    ScriptError,
    CyclesExceeded,
    MemoryExceeded,
    Timeout,
    UnknownSyscall,
    ResultTooLarge,
    InvalidElf,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::ScriptError => "script_error",
            Status::CyclesExceeded => "cycles_exceeded",
            Status::MemoryExceeded => "memory_exceeded",
            Status::Timeout => "timeout",
            Status::UnknownSyscall => "unknown_syscall",
            Status::ResultTooLarge => "result_too_large",
            Status::InvalidElf => "invalid_elf",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_cycles: u64,
    pub max_memory_mb: u32,
    pub max_wall_seconds: u32,
}

pub struct Outcome {
    pub status: Status,
    pub exit_code: i32,
    pub cycles: u64,
    pub result: Vec<u8>,
    pub debug: Vec<String>,
    /// Human-readable VM error, for stderr; not part of the JSON.
    pub error: Option<String>,
}

/// Maps the VM's end state to a status and exit code.
pub fn classify(r: &Result<i8, Error>) -> (Status, i32) {
    match r {
        Ok(0) => (Status::Ok, 0),
        Ok(c) => (Status::ScriptError, i32::from(*c)),
        Err(Error::CyclesExceeded) => (Status::CyclesExceeded, -1),
        Err(Error::MemOutOfBound | Error::MemOutOfStack) => (Status::MemoryExceeded, -1),
        Err(Error::Pause) => (Status::Timeout, -1),
        Err(Error::InvalidEcall(_)) => (Status::UnknownSyscall, -1),
        Err(Error::External(t)) if t == host::STOP_RESULT_TOO_LARGE => (Status::ResultTooLarge, -1),
        Err(_) => (Status::ScriptError, -1),
    }
}

fn is_invalid_elf(e: &Error) -> bool {
    matches!(
        e,
        Error::ElfBits
            | Error::ElfParseError(_)
            | Error::ElfSegmentUnreadable
            | Error::ElfSegmentWritableAndExecutable
            | Error::ElfSegmentAddrOrSizeError
    )
}

/// Runs `elf` against the virtual transaction in `state`.
pub fn run(elf: &[u8], state: HostState, limits: Limits) -> Result<(Outcome, HostState), String> {
    if limits.max_memory_mb != VM_MEMORY_MB {
        return Err(format!(
            "only {VM_MEMORY_MB} MiB of VM memory is supported (got {})",
            limits.max_memory_mb
        ));
    }
    let state = Arc::new(Mutex::new(state));
    let pause = Pause::new();
    // Same ISA and version as CKB uses for scripts with `hash_type = data2`.
    let core = <Box<AsmCoreMachine> as SupportMachine>::new(
        ISA_IMC | ISA_B | ISA_MOP,
        VERSION2,
        limits.max_cycles,
    );
    let dm = DefaultMachineBuilder::new(core)
        .instruction_cycle_func(Box::new(estimate_cycles))
        .syscall(Box::new(Host {
            state: state.clone(),
        }))
        .pause(pause.clone())
        .build();
    let mut machine = AsmMachine::new(dm);
    let program = Bytes::copy_from_slice(elf);
    let finish =
        |status, exit_code, cycles, error: Option<String>, state: Arc<Mutex<HostState>>| {
            let st = Arc::try_unwrap(state).ok().map(|m| m.into_inner().unwrap());
            st.map(|st| {
                let outcome = Outcome {
                    status,
                    exit_code,
                    cycles,
                    result: if status == Status::Ok || status == Status::ScriptError {
                        st.final_result()
                    } else {
                        Vec::new()
                    },
                    debug: st.debug.clone(),
                    error,
                };
                (outcome, st)
            })
        };
    if let Err(e) = machine.load_program(&program, std::iter::empty::<Result<Bytes, Error>>()) {
        let status = if is_invalid_elf(&e) {
            Status::InvalidElf
        } else {
            Status::ScriptError
        };
        drop(machine);
        return finish(status, -1, 0, Some(e.to_string()), state)
            .ok_or_else(|| "host state still in use".to_string());
    }

    // Wall-time watchdog: interrupts the VM through its pause flag, which also stops loops
    // that never enter a syscall.
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let wd_pause = pause.clone();
    let wall = Duration::from_secs(u64::from(limits.max_wall_seconds));
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(wall) == Err(mpsc::RecvTimeoutError::Timeout) {
            wd_pause.interrupt();
        }
    });
    let r = machine.run();
    drop(done_tx);
    let _ = watchdog.join();

    let cycles = machine.machine.cycles();
    let (status, exit_code) = classify(&r);
    let error = r.as_ref().err().map(|e| e.to_string());
    drop(machine);
    finish(status, exit_code, cycles, error, state)
        .ok_or_else(|| "host state still in use".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        assert_eq!(classify(&Ok(0)), (Status::Ok, 0));
        assert_eq!(classify(&Ok(3)), (Status::ScriptError, 3));
        assert_eq!(classify(&Ok(-2)), (Status::ScriptError, -2));
        assert_eq!(
            classify(&Err(Error::CyclesExceeded)).0,
            Status::CyclesExceeded
        );
        assert_eq!(
            classify(&Err(Error::MemOutOfBound)).0,
            Status::MemoryExceeded
        );
        assert_eq!(
            classify(&Err(Error::MemOutOfStack)).0,
            Status::MemoryExceeded
        );
        assert_eq!(classify(&Err(Error::Pause)).0, Status::Timeout);
        assert_eq!(
            classify(&Err(Error::InvalidEcall(9999))).0,
            Status::UnknownSyscall
        );
        assert_eq!(
            classify(&Err(Error::External(host::STOP_RESULT_TOO_LARGE.into()))).0,
            Status::ResultTooLarge
        );
        assert_eq!(
            classify(&Err(Error::InvalidInstruction {
                pc: 0,
                instruction: 0
            }))
            .0,
            Status::ScriptError
        );
    }

    #[test]
    fn status_names() {
        assert_eq!(Status::CyclesExceeded.as_str(), "cycles_exceeded");
        assert_eq!(Status::InvalidElf.as_str(), "invalid_elf");
        assert_eq!(Status::ResultTooLarge.as_str(), "result_too_large");
    }

    #[test]
    fn invalid_elf_classification() {
        assert!(is_invalid_elf(&Error::ElfParseError("x".into())));
        assert!(!is_invalid_elf(&Error::CyclesExceeded));
    }

    #[test]
    fn rejects_other_memory_sizes() {
        let st = HostState::new(vtx::VirtualTx::new(vec![], vec![], vec![], vec![]), [0; 3]);
        let limits = Limits {
            max_cycles: 1,
            max_memory_mb: 8,
            max_wall_seconds: 1,
        };
        assert!(run(b"x", st, limits).is_err());
    }

    #[test]
    fn garbage_is_invalid_elf() {
        let st = HostState::new(
            vtx::VirtualTx::new(b"garbage".to_vec(), vec![], vec![], vec![]),
            [0; 3],
        );
        let limits = Limits {
            max_cycles: 1000,
            max_memory_mb: 4,
            max_wall_seconds: 1,
        };
        let (o, _) = run(b"garbage garbage garbage", st, limits).unwrap();
        assert_eq!(o.status, Status::InvalidElf);
    }
}
