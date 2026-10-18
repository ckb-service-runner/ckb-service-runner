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
             );",
        )
        .map_err(|e| format!("cannot initialise database: {e}"))?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
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
    fn keeps_24_hours() {
        let db = Db::open_memory();
        db.add_sample(&s(1000)).unwrap();
        db.add_sample(&s(1000 + KEEP_SECONDS + 5)).unwrap();
        let all = db.samples_since(0).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].t, 1000 + KEEP_SECONDS + 5);
    }
}
