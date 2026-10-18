//! Resource monitor (CPU, RAM, disk) and the "node always wins" hold conditions.

use crate::config::LimitsCfg;
use crate::node::r2::R2State;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use sysinfo::{Disks, Pid, ProcessesToUpdate, System};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Cpu {
    pub total_percent: f64,
    pub csr_percent: f64,
    pub ckb_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Ram {
    pub total_mb: u64,
    pub free_mb: u64,
    pub csr_mb: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Disk {
    pub free_gb: u64,
    pub total_gb: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Resources {
    pub cpu: Cpu,
    pub ram: Ram,
    pub disk: Disk,
}

pub struct Monitor {
    sys: Mutex<System>,
    data_root: PathBuf,
}

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

impl Monitor {
    pub fn new(data_root: PathBuf) -> Monitor {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        Monitor {
            sys: Mutex::new(sys),
            data_root: data_root.canonicalize().unwrap_or(data_root),
        }
    }

    /// Refreshes the counters and returns the current values. CPU values are
    /// averages since the previous call, as a percentage of the whole machine.
    pub fn sample(&self) -> Resources {
        let mut sys = self.sys.lock().unwrap();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let cores = sys.cpus().len().max(1) as f64;
        let me = Pid::from_u32(std::process::id());
        let (csr_percent, csr_mb) = sys
            .process(me)
            .map(|p| (f64::from(p.cpu_usage()) / cores, p.memory() / MB))
            .unwrap_or((0.0, 0));
        let ckb: Vec<f64> = sys
            .processes()
            .values()
            .filter(|p| p.name() == "ckb")
            .map(|p| f64::from(p.cpu_usage()) / cores)
            .collect();
        let ckb_percent = if ckb.is_empty() {
            None
        } else {
            Some(round1(ckb.iter().sum()))
        };
        let disks = Disks::new_with_refreshed_list();
        let disk = disks
            .list()
            .iter()
            .filter(|d| self.data_root.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .map(|d| Disk {
                free_gb: d.available_space() / GB,
                total_gb: d.total_space() / GB,
            })
            .unwrap_or(Disk {
                free_gb: 0,
                total_gb: 0,
            });
        Resources {
            cpu: Cpu {
                total_percent: round1(f64::from(sys.global_cpu_usage())),
                csr_percent: round1(csr_percent),
                ckb_percent,
            },
            ram: Ram {
                total_mb: sys.total_memory() / MB,
                free_mb: sys.available_memory() / MB,
                csr_mb,
            },
            disk,
        }
    }
}

/// "Node always wins": returns why new jobs must not start (and running jobs
/// must pause), or `None` when the machine has room. Used by the job scheduler
/// in a later milestone.
#[allow(dead_code)]
pub fn hold_reason(limits: &LimitsCfg, r: &Resources, r2: R2State) -> Option<String> {
    if r2 != R2State::Synced {
        return Some(format!("node not synced (R2: {})", r2.as_str()));
    }
    if r.cpu.total_percent > f64::from(limits.pause_above_cpu_percent) {
        return Some(format!(
            "cpu {:.0}% above limit {}%",
            r.cpu.total_percent, limits.pause_above_cpu_percent
        ));
    }
    if r.ram.free_mb < limits.min_free_ram_mb {
        return Some(format!(
            "free RAM {} MB below minimum {} MB",
            r.ram.free_mb, limits.min_free_ram_mb
        ));
    }
    if r.disk.free_gb < limits.min_free_disk_gb {
        return Some(format!(
            "free disk {} GB below minimum {} GB",
            r.disk.free_gb, limits.min_free_disk_gb
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(cpu: f64, ram: u64, disk: u64) -> Resources {
        Resources {
            cpu: Cpu {
                total_percent: cpu,
                csr_percent: 0.0,
                ckb_percent: None,
            },
            ram: Ram {
                total_mb: 8000,
                free_mb: ram,
                csr_mb: 10,
            },
            disk: Disk {
                free_gb: disk,
                total_gb: 500,
            },
        }
    }

    #[test]
    fn hold_conditions() {
        let l = LimitsCfg::default();
        assert_eq!(
            hold_reason(&l, &res(10.0, 4000, 100), R2State::Synced),
            None
        );
        assert!(hold_reason(&l, &res(10.0, 4000, 100), R2State::Syncing).is_some());
        assert!(hold_reason(&l, &res(90.0, 4000, 100), R2State::Synced)
            .unwrap()
            .contains("cpu"));
        assert_eq!(
            hold_reason(&l, &res(85.0, 4000, 100), R2State::Synced),
            None
        );
        assert!(hold_reason(&l, &res(10.0, 1023, 100), R2State::Synced)
            .unwrap()
            .contains("RAM"));
        assert!(hold_reason(&l, &res(10.0, 4000, 9), R2State::Synced)
            .unwrap()
            .contains("disk"));
    }

    #[test]
    fn sampling_works() {
        let m = Monitor::new(std::env::current_dir().unwrap());
        let r = m.sample();
        assert!(r.ram.total_mb > 0);
    }
}
