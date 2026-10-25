//! SQLite state: settings state, metric samples, services, job cells, job runs and settlements.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

const KEEP_SECONDS: i64 = 24 * 3600;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Sample {
    pub t: i64,
    pub cpu_percent: f64,
    pub ram_free_mb: u64,
    pub tip_lag: Option<u64>,
    pub disk_free_gb: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ServiceRow {
    pub code_hash: String,
    pub name: String,
    pub language: String,
    pub status: String,
    /// Code cell of an on-chain service (`0x<tx>:<index>`).
    pub outpoint: Option<String>,
}

/// One run of a job cell (`keeper.rs` has the states).
#[derive(Debug, Clone, PartialEq)]
pub struct JobRun {
    pub id: i64,
    pub outpoint: String,
    pub code_hash: String,
    pub trigger: String,
    pub tip_shannons: u64,
    pub max_cycles: u64,
    pub tip_rate: f64,
    pub state: String,
    pub cycles: Option<u64>,
    pub result_tx: Option<String>,
    pub result_hex: Option<String>,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub submit_block: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub t: i64,
    pub state: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settlement {
    pub job_outpoint: String,
    pub run_id: i64,
    pub tx_hash: String,
    pub tip_shannons: u64,
    pub block: u64,
    pub t: i64,
}

const RUN_COLS: &str = "id, outpoint, code_hash, trigger, tip_shannons, max_cycles, tip_rate, state, cycles, result_tx, result_hex, error, logs, submit_block";

fn run_row(r: &rusqlite::Row) -> rusqlite::Result<JobRun> {
    let logs: String = r.get(12)?;
    Ok(JobRun {
        id: r.get(0)?,
        outpoint: r.get(1)?,
        code_hash: r.get(2)?,
        trigger: r.get(3)?,
        tip_shannons: r.get::<_, i64>(4)? as u64,
        max_cycles: r.get::<_, i64>(5)? as u64,
        tip_rate: r.get(6)?,
        state: r.get(7)?,
        cycles: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        result_tx: r.get(9)?,
        result_hex: r.get(10)?,
        error: r.get(11)?,
        logs: serde_json::from_str(&logs).unwrap_or_default(),
        submit_block: r.get::<_, Option<i64>>(13)?.map(|v| v as u64),
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A discovered job cell. `state` is `discovered` (eligible) or `excluded` (with `reason`).
#[derive(Debug, Clone, PartialEq)]
pub struct JobCellRow {
    pub outpoint: String,
    pub code_hash: String,
    pub tip_shannons: u64,
    pub max_cycles: u64,
    pub trigger: String,
    pub state: String,
    pub reason: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db, String> {
        let conn = Connection::open(path).map_err(|e| format!("cannot open database: {e}"))?;
        Db::init(conn)
    }

    #[cfg(test)]
    pub fn open_memory() -> Db {
        Db::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> Result<Db, String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS samples (
                 t INTEGER PRIMARY KEY,
                 cpu_percent REAL NOT NULL,
                 ram_free_mb INTEGER NOT NULL,
                 tip_lag INTEGER,
                 disk_free_gb INTEGER
             );
             CREATE TABLE IF NOT EXISTS services (
                 code_hash TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 language TEXT NOT NULL,
                 status TEXT NOT NULL DEFAULT 'not_approved'
             );
             CREATE TABLE IF NOT EXISTS job_cells (
                 outpoint TEXT PRIMARY KEY,
                 code_hash TEXT NOT NULL,
                 tip_shannons INTEGER NOT NULL,
                 max_cycles INTEGER NOT NULL,
                 trigger TEXT NOT NULL,
                 state TEXT NOT NULL,
                 reason TEXT,
                 seen_block INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS job_runs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 outpoint TEXT NOT NULL,
                 code_hash TEXT NOT NULL,
                 trigger TEXT NOT NULL,
                 tip_shannons INTEGER NOT NULL,
                 max_cycles INTEGER NOT NULL,
                 tip_rate REAL NOT NULL,
                 state TEXT NOT NULL,
                 cycles INTEGER,
                 result_tx TEXT,
                 result_hex TEXT,
                 error TEXT,
                 logs TEXT NOT NULL DEFAULT '[]',
                 submit_block INTEGER
             );
             CREATE INDEX IF NOT EXISTS job_runs_outpoint ON job_runs (outpoint);
             CREATE TABLE IF NOT EXISTS job_events (
                 run_id INTEGER NOT NULL,
                 t INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 note TEXT
             );
             CREATE TABLE IF NOT EXISTS settlements (
                 job_outpoint TEXT PRIMARY KEY,
                 run_id INTEGER NOT NULL,
                 tx_hash TEXT NOT NULL,
                 tip_shannons INTEGER NOT NULL,
                 block INTEGER NOT NULL,
                 t INTEGER NOT NULL
             );",
        )
        .map_err(|e| format!("cannot initialise database: {e}"))?;
        // Added in A3c; older databases get the column here.
        let has_outpoint = conn
            .prepare("SELECT outpoint FROM services LIMIT 0")
            .is_ok();
        if !has_outpoint {
            conn.execute_batch("ALTER TABLE services ADD COLUMN outpoint TEXT")
                .map_err(|e| format!("cannot migrate database: {e}"))?;
        }
        let has_disk = conn
            .prepare("SELECT disk_free_gb FROM samples LIMIT 0")
            .is_ok();
        if !has_disk {
            conn.execute_batch("ALTER TABLE samples ADD COLUMN disk_free_gb INTEGER")
                .map_err(|e| format!("cannot migrate database: {e}"))?;
        }
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    /// Registers a service; an already known code hash keeps its status.
    pub fn add_service(&self, code_hash: &str, name: &str, language: &str) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO services (code_hash, name, language) VALUES (?1, ?2, ?3)
             ON CONFLICT(code_hash) DO UPDATE SET name = ?2, language = ?3",
            params![code_hash, name, language],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Records the code cell of an on-chain service.
    pub fn set_service_outpoint(&self, code_hash: &str, outpoint: &str) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "UPDATE services SET outpoint = ?2 WHERE code_hash = ?1",
            params![code_hash, outpoint],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn services(&self) -> Result<Vec<ServiceRow>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT code_hash, name, language, status, outpoint FROM services ORDER BY name, code_hash",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                Ok(ServiceRow {
                    code_hash: r.get(0)?,
                    name: r.get(1)?,
                    language: r.get(2)?,
                    status: r.get(3)?,
                    outpoint: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn service(&self, code_hash: &str) -> Result<Option<ServiceRow>, String> {
        Ok(self
            .services()?
            .into_iter()
            .find(|s| s.code_hash == code_hash))
    }

    /// Sets the status of a known service; false if the code hash is unknown.
    pub fn set_service_status(&self, code_hash: &str, status: &str) -> Result<bool, String> {
        let c = self.conn.lock().unwrap();
        let n = c
            .execute(
                "UPDATE services SET status = ?2 WHERE code_hash = ?1",
                params![code_hash, status],
            )
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    /// Replaces the set of discovered job cells by the live cells seen at `block`.
    pub fn replace_job_cells(&self, rows: &[JobCellRow], block: u64) -> Result<(), String> {
        let mut c = self.conn.lock().unwrap();
        let tx = c.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM job_cells", [])
            .map_err(|e| e.to_string())?;
        for r in rows {
            tx.execute(
                "INSERT INTO job_cells (outpoint, code_hash, tip_shannons, max_cycles, trigger, state, reason, seen_block)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    r.outpoint,
                    r.code_hash,
                    r.tip_shannons as i64,
                    r.max_cycles as i64,
                    r.trigger,
                    r.state,
                    r.reason,
                    block as i64
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    #[allow(dead_code)] // read by the jobs API
    pub fn job_cells(&self) -> Result<Vec<JobCellRow>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT outpoint, code_hash, tip_shannons, max_cycles, trigger, state, reason FROM job_cells ORDER BY outpoint",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                Ok(JobCellRow {
                    outpoint: r.get(0)?,
                    code_hash: r.get(1)?,
                    tip_shannons: r.get::<_, i64>(2)? as u64,
                    max_cycles: r.get::<_, i64>(3)? as u64,
                    trigger: r.get(4)?,
                    state: r.get(5)?,
                    reason: r.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    /// Creates a run in `state`; returns its id.
    pub fn add_run(&self, r: &JobRun) -> Result<i64, String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO job_runs (outpoint, code_hash, trigger, tip_shannons, max_cycles, tip_rate, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                r.outpoint,
                r.code_hash,
                r.trigger,
                r.tip_shannons as i64,
                r.max_cycles as i64,
                r.tip_rate,
                r.state
            ],
        )
        .map_err(|e| e.to_string())?;
        let id = c.last_insert_rowid();
        c.execute(
            "INSERT INTO job_events (run_id, t, state, note) VALUES (?1, ?2, ?3, NULL)",
            params![id, now(), r.state],
        )
        .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Moves a run to `state` and records the transition in its timeline.
    pub fn set_run_state(&self, id: i64, state: &str, note: Option<&str>) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "UPDATE job_runs SET state = ?2 WHERE id = ?1",
            params![id, state],
        )
        .map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO job_events (run_id, t, state, note) VALUES (?1, ?2, ?3, ?4)",
            params![id, now(), state, note],
        )
        .map_err(|e| e.to_string())?;
        if let (Some(n), true) = (
            note,
            ["Failed", "Rejected", "Lost", "Expired"].contains(&state),
        ) {
            c.execute(
                "UPDATE job_runs SET error = ?2 WHERE id = ?1",
                params![id, n],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Stores the outcome of the VM run.
    pub fn set_run_result(
        &self,
        id: i64,
        cycles: u64,
        result_hex: &str,
        logs: &[String],
    ) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "UPDATE job_runs SET cycles = ?2, result_hex = ?3, logs = ?4 WHERE id = ?1",
            params![
                id,
                cycles as i64,
                result_hex,
                serde_json::to_string(logs).unwrap_or_else(|_| "[]".into())
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_run_tx(&self, id: i64, tx_hash: &str, block: u64) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "UPDATE job_runs SET result_tx = ?2, submit_block = ?3 WHERE id = ?1",
            params![id, tx_hash, block as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// All runs, newest first.
    pub fn runs(&self) -> Result<Vec<JobRun>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(&format!("SELECT {RUN_COLS} FROM job_runs ORDER BY id DESC"))
            .map_err(|e| e.to_string())?;
        let rows = st.query_map([], run_row).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn runs_in(&self, states: &[&str]) -> Result<Vec<JobRun>, String> {
        Ok(self
            .runs()?
            .into_iter()
            .rev()
            .filter(|r| states.contains(&r.state.as_str()))
            .collect())
    }

    pub fn run(&self, id: i64) -> Result<Option<JobRun>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(&format!("SELECT {RUN_COLS} FROM job_runs WHERE id = ?1"))
            .map_err(|e| e.to_string())?;
        let mut rows = st.query_map([id], run_row).map_err(|e| e.to_string())?;
        rows.next().transpose().map_err(|e| e.to_string())
    }

    pub fn events(&self, id: i64) -> Result<Vec<Event>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare("SELECT t, state, note FROM job_events WHERE run_id = ?1 ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([id], |r| {
                Ok(Event {
                    t: r.get(0)?,
                    state: r.get(1)?,
                    note: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    /// On restart, runs that were interrupted go back to `Queued`.
    pub fn requeue_interrupted(&self) -> Result<usize, String> {
        let ids: Vec<i64> = self
            .runs_in(&["Running", "Scheduled", "Paused", "Completed"])?
            .iter()
            .map(|r| r.id)
            .collect();
        for id in &ids {
            self.set_run_state(*id, "Queued", Some("restart"))?;
        }
        Ok(ids.len())
    }

    /// Records a settlement verified on chain and marks the run `Settled`.
    pub fn settle(&self, s: &Settlement) -> Result<(), String> {
        {
            let c = self.conn.lock().unwrap();
            c.execute(
                "INSERT OR IGNORE INTO settlements (job_outpoint, run_id, tx_hash, tip_shannons, block, t)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    s.job_outpoint,
                    s.run_id,
                    s.tx_hash,
                    s.tip_shannons as i64,
                    s.block as i64,
                    s.t
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        self.set_run_state(s.run_id, "Settled", Some(&format!("block {}", s.block)))
    }

    /// Settlements, newest first.
    pub fn settlements(&self) -> Result<Vec<Settlement>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare("SELECT job_outpoint, run_id, tx_hash, tip_shannons, block, t FROM settlements ORDER BY block DESC, t DESC")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                Ok(Settlement {
                    job_outpoint: r.get(0)?,
                    run_id: r.get(1)?,
                    tx_hash: r.get(2)?,
                    tip_shannons: r.get::<_, i64>(3)? as u64,
                    block: r.get::<_, i64>(4)? as u64,
                    t: r.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = st
            .query_map([key], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.next().transpose().map_err(|e| e.to_string())
    }

    pub fn put_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Stores a sample and drops everything older than 24 h.
    pub fn add_sample(&self, s: &Sample) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT OR REPLACE INTO samples (t, cpu_percent, ram_free_mb, tip_lag, disk_free_gb) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![s.t, s.cpu_percent, s.ram_free_mb as i64, s.tip_lag.map(|v| v as i64), s.disk_free_gb.map(|v| v as i64)],
        )
        .map_err(|e| e.to_string())?;
        c.execute(
            "DELETE FROM samples WHERE t < ?1",
            params![s.t - KEEP_SECONDS],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Per `job_runs.code_hash`: number of settlements and the sum of their tips.
    pub fn service_totals(&self) -> Result<std::collections::HashMap<String, (u64, u64)>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT j.code_hash, COUNT(*), COALESCE(SUM(s.tip_shannons), 0)
                 FROM settlements s JOIN job_runs j ON j.id = s.run_id GROUP BY j.code_hash",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64),
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn samples_since(&self, since: i64) -> Result<Vec<Sample>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT t, cpu_percent, ram_free_mb, tip_lag, disk_free_gb FROM samples WHERE t >= ?1 ORDER BY t",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![since], |r| {
                Ok(Sample {
                    t: r.get(0)?,
                    cpu_percent: r.get(1)?,
                    ram_free_mb: r.get::<_, i64>(2)? as u64,
                    tip_lag: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    disk_free_gb: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "../../acceptance/aiworker-csr-runner-fixes/runner_fixes_tests.rs"]
mod aiworker_runner_fixes_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: i64) -> Sample {
        Sample {
            t,
            cpu_percent: 1.5,
            ram_free_mb: 100,
            tip_lag: Some(2),
            disk_free_gb: Some(50),
        }
    }

    #[test]
    fn service_registry() {
        let db = Db::open_memory();
        db.add_service("0xaa", "x", "rust").unwrap();
        db.set_service_status("0xaa", "approved_enabled").unwrap();
        db.add_service("0xaa", "y", "rust").unwrap();
        let s = db.service("0xaa").unwrap().unwrap();
        assert_eq!(
            (s.name.as_str(), s.status.as_str()),
            ("y", "approved_enabled")
        );
        assert!(!db.set_service_status("0xbb", "not_approved").unwrap());
    }

    #[test]
    fn job_cells_are_replaced() {
        let db = Db::open_memory();
        let row = |op: &str| JobCellRow {
            outpoint: op.into(),
            code_hash: "0xaa".into(),
            tip_shannons: 5,
            max_cycles: 6,
            trigger: "ondemand".into(),
            state: "discovered".into(),
            reason: None,
        };
        db.replace_job_cells(&[row("a"), row("b")], 1).unwrap();
        db.replace_job_cells(&[row("b")], 2).unwrap();
        assert_eq!(db.job_cells().unwrap(), vec![row("b")]);
    }

    #[test]
    fn run_lifecycle_and_settlement() {
        let db = Db::open_memory();
        let run = JobRun {
            id: 0,
            outpoint: "0x01:0".into(),
            code_hash: "0xaa".into(),
            trigger: "ondemand".into(),
            tip_shannons: 5,
            max_cycles: 6,
            tip_rate: 1.0,
            state: "Queued".into(),
            cycles: None,
            result_tx: None,
            result_hex: None,
            error: None,
            logs: vec![],
            submit_block: None,
        };
        let id = db.add_run(&run).unwrap();
        db.set_run_state(id, "Running", None).unwrap();
        assert_eq!(db.requeue_interrupted().unwrap(), 1);
        assert_eq!(db.run(id).unwrap().unwrap().state, "Queued");
        db.set_run_result(id, 100, "0x01", &["line".into()])
            .unwrap();
        db.set_run_tx(id, "0xbeef", 7).unwrap();
        db.settle(&Settlement {
            job_outpoint: "0x01:0".into(),
            run_id: id,
            tx_hash: "0xbeef".into(),
            tip_shannons: 5,
            block: 8,
            t: 1,
        })
        .unwrap();
        let r = db.run(id).unwrap().unwrap();
        assert_eq!(
            (r.state.as_str(), r.cycles, r.logs.len()),
            ("Settled", Some(100), 1)
        );
        let states: Vec<_> = db
            .events(id)
            .unwrap()
            .into_iter()
            .map(|e| e.state)
            .collect();
        assert_eq!(states, ["Queued", "Running", "Queued", "Settled"]);
        assert_eq!(db.settlements().unwrap().len(), 1);
        db.put_setting("k", "v").unwrap();
        assert_eq!(db.get_setting("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn keeps_24_hours() {
        let db = Db::open_memory();
        db.add_sample(&s(1000)).unwrap();
        db.add_sample(&s(1000 + KEEP_SECONDS + 5)).unwrap();
        let all = db.samples_since(0).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].t, 1000 + KEEP_SECONDS + 5);
    }
}
