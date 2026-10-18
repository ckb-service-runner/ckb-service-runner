//! Dashboard password: argon2id hashing and `csr set-password`.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use std::io::{BufRead, Write};
use std::path::Path;

pub fn hash(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("hashing failed: {e}"))
}

pub fn verify(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

fn read_line() -> Result<String, String> {
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read password: {e}"))?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

fn prompt(label: &str) -> Result<String, String> {
    eprint!("{label}");
    let _ = std::io::stderr().flush();
    // Best effort: switch off terminal echo while the password is typed.
    let stty = |arg: &str| {
        let _ = std::process::Command::new("stty")
            .arg(arg)
            .stdin(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::null())
            .status();
    };
    stty("-echo");
    let r = read_line();
    stty("echo");
    eprintln!();
    r
}

/// Stores an argon2id hash of a new password in the config file.
pub fn set_password(config_path: &Path, from_stdin: bool) -> Result<(), String> {
    let pw = if from_stdin {
        read_line()?
    } else {
        let a = prompt("New dashboard password: ")?;
        let b = prompt("Repeat password: ")?;
        if a != b {
            return Err("passwords do not match".into());
        }
        a
    };
    if pw.is_empty() {
        return Err("the password must not be empty".into());
    }
    let mut cfg = crate::config::Config::load_unvalidated(config_path)?;
    cfg.dashboard.password_hash = hash(&pw)?;
    cfg.validate()?;
    cfg.save(config_path)?;
    eprintln!("password stored in {}", config_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash("secret").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify("secret", &h));
        assert!(!verify("other", &h));
        assert!(!verify("secret", ""));
    }
}
