//! SQLite state: settings state and metric samples.

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
}

#[derive(Debug, Clone)]
pub struct ServiceRow {
    pub code_hash: String,
    pub name: String,
    pub language: String,
    pub status: String,
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
                 tip_lag INTEGER
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
             );",
        )
        .map_err(|e| format!("cannot initialise database: {e}"))?;
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

    pub fn services(&self) -> Result<Vec<ServiceRow>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT code_hash, name, language, status FROM services ORDER BY name, code_hash",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([], |r| {
                Ok(ServiceRow {
                    code_hash: r.get(0)?,
                    name: r.get(1)?,
                    language: r.get(2)?,
                    status: r.get(3)?,
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

    /// Stores a sample and drops everything older than 24 h.
    pub fn add_sample(&self, s: &Sample) -> Result<(), String> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT OR REPLACE INTO samples (t, cpu_percent, ram_free_mb, tip_lag) VALUES (?1, ?2, ?3, ?4)",
            params![s.t, s.cpu_percent, s.ram_free_mb as i64, s.tip_lag.map(|v| v as i64)],
        )
        .map_err(|e| e.to_string())?;
        c.execute(
            "DELETE FROM samples WHERE t < ?1",
            params![s.t - KEEP_SECONDS],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn samples_since(&self, since: i64) -> Result<Vec<Sample>, String> {
        let c = self.conn.lock().unwrap();
        let mut st = c
            .prepare(
                "SELECT t, cpu_percent, ram_free_mb, tip_lag FROM samples WHERE t >= ?1 ORDER BY t",
            )
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map(params![since], |r| {
                Ok(Sample {
                    t: r.get(0)?,
                    cpu_percent: r.get(1)?,
                    ram_free_mb: r.get::<_, i64>(2)? as u64,
                    tip_lag: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: i64) -> Sample {
        Sample {
            t,
            cpu_percent: 1.5,
            ram_free_mb: 100,
            tip_lag: Some(2),
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
    fn keeps_24_hours() {
        let db = Db::open_memory();
        db.add_sample(&s(1000)).unwrap();
        db.add_sample(&s(1000 + KEEP_SECONDS + 5)).unwrap();
        let all = db.samples_since(0).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].t, 1000 + KEEP_SECONDS + 5);
    }
}
