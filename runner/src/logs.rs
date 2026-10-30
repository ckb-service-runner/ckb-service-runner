//! File + stderr logging and reading the log back for the dashboard.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const TAIL_BYTES: u64 = 512 * 1024;
pub const MAX_LINES: usize = 1000;

/// A log file that moves to `<name>.log.1` (replacing an older one) before a write would take it over
/// `max_bytes`, and starts a new file.
pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    file: File,
    size: u64,
}

fn rotated_path(path: &Path) -> PathBuf {
    let mut old = path.to_path_buf();
    old.set_extension("log.1");
    old
}

impl RotatingFile {
    pub fn open(path: &Path, max_bytes: u64) -> io::Result<RotatingFile> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let size = file.metadata()?.len();
        Ok(RotatingFile {
            path: path.to_path_buf(),
            max_bytes,
            file,
            size,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        fs::rename(&self.path, rotated_path(&self.path))?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.size = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let n = self.file.write(buf)?;
        self.size += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[derive(Clone)]
struct Tee {
    file: Arc<Mutex<RotatingFile>>,
}

struct TeeWriter(Tee);

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        let _ = self.0.file.lock().unwrap().write_all(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Tee {
    type Writer = TeeWriter;
    fn make_writer(&'a self) -> TeeWriter {
        TeeWriter(self.clone())
    }
}

/// Changes the log level of the running process.
pub type LevelSetter = Box<dyn Fn(&str) + Send + Sync>;

fn targets(level: &str) -> Targets {
    let l: LevelFilter = level.parse().unwrap_or(LevelFilter::INFO);
    Targets::new()
        .with_default(LevelFilter::WARN)
        .with_target("csr", l)
}

/// Starts logging to `<path>` and stderr. Returns a setter for the level.
pub fn init(path: &Path, level: &str) -> Result<LevelSetter, String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let file = RotatingFile::open(path, MAX_LOG_BYTES)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let (filter, handle) = reload::Layer::new(targets(level));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Tee {
                    file: Arc::new(Mutex::new(file)),
                }),
        )
        .try_init()
        .map_err(|e| format!("cannot start logging: {e}"))?;
    Ok(Box::new(move |lvl| {
        let _ = handle.reload(targets(lvl));
    }))
}

fn rank(level: &str) -> Option<u8> {
    match level.to_ascii_lowercase().as_str() {
        "error" => Some(0),
        "warn" => Some(1),
        "info" => Some(2),
        "debug" => Some(3),
        "trace" => Some(4),
        _ => None,
    }
}

pub fn valid_level(level: &str) -> bool {
    rank(level).is_some()
}

/// The last `max` bytes of a file and the offset they start at.
fn read_tail(path: &Path, max: u64) -> io::Result<(Vec<u8>, u64)> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max);
    let mut buf = Vec::new();
    f.seek(SeekFrom::Start(start))?;
    f.read_to_end(&mut buf)?;
    Ok((buf, start))
}

/// Last lines of the log with a level at least as severe as `level`
/// (at most 1,000).
pub fn tail(path: &Path, level: &str) -> Vec<String> {
    let Some(max) = rank(level) else {
        return vec![];
    };
    let Ok((buf, start)) = read_tail(path, TAIL_BYTES) else {
        return vec![];
    };
    // Just after a rotation the current file is short: continue with the end of the rotated one.
    let (buf, start) = if start == 0 && (buf.len() as u64) < TAIL_BYTES {
        match read_tail(&rotated_path(path), TAIL_BYTES - buf.len() as u64) {
            Ok((mut old, ostart)) if !old.is_empty() => {
                if ostart > 0 {
                    match old.iter().position(|b| *b == b'\n') {
                        Some(i) => {
                            old.drain(..=i);
                        }
                        None => old.clear(),
                    }
                }
                old.extend_from_slice(&buf);
                (old, 0)
            }
            _ => (buf, start),
        }
    } else {
        (buf, start)
    };
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // probably cut in the middle
    }
    let mut out: Vec<String> = lines
        .into_iter()
        .filter(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(rank)
                .map(|r| r <= max)
                .unwrap_or(true)
        })
        .map(String::from)
        .collect();
    if out.len() > MAX_LINES {
        out.drain(..out.len() - MAX_LINES);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_filters_by_level() {
        let tmp = crate::test_tmp();
        let p = tmp.path().join("csr.log");
        fs::write(
            &p,
            "2026-01-01T00:00:00Z  INFO csr: a\n2026-01-01T00:00:01Z DEBUG csr: b\n2026-01-01T00:00:02Z ERROR csr: c\n",
        )
        .unwrap();
        assert_eq!(tail(&p, "info").len(), 2);
        assert_eq!(tail(&p, "debug").len(), 3);
        assert_eq!(tail(&p, "error").len(), 1);
        assert!(tail(&p, "bogus").is_empty());
    }
}
