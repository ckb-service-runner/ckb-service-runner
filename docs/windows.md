# CSR on Windows (beta)

x86_64 only, unsigned, no auto-update (update = run `install.ps1` of the new zip again). Data root
`%ProgramData%\csr`, binary `%ProgramFiles%\csr\csr.exe`. Behaviour, ports and the mainnet lock are the same as on Linux.

## Porting notes

Unix-only uses found in `runner/` and how phase 1 gates them (`csr` depends on no other workspace crate):

- `runner/Cargo.toml`: `libc` is now a `cfg(unix)` dependency.
- `runner/src/main.rs`: `libc::umask(0o077)` at start – `#[cfg(unix)]`, no-op on Windows.
- `runner/src/main.rs`: default data root `$HOME/.csr` – now `default_data_root()`: Unix unchanged, Windows `%ProgramData%\csr` (fallback `C:\ProgramData\csr`). `--data-root` still wins.
- `runner/src/run.rs`: `tokio::signal::unix` SIGTERM/SIGINT – Unix unchanged; Windows waits for Ctrl-C / Ctrl-Break.
- `runner/src/identity.rs`: `OpenOptionsExt::mode(0o600)` for new key files – Unix only.
- `runner/src/identity.rs`: key file chmod to 0600 when too open (`key_mode_too_open`) and `keys/` dir 0700 – Unix only.
- `runner/src/identity.rs`: `node.p2p_key_file` mode check (refuse "other"-readable) – Unix unchanged; Windows only checks that the file exists and is readable and logs one `info` line that permission checks are skipped.
- `runner/src/config.rs`: `Config::save` created the temp file with mode 0600 – Unix only; Windows relies on the ACL of `%ProgramData%\csr`.
- `runner/src/update.rs`: `write_file` set file modes (0755/0644) on staged update files – Unix only.
- `runner/src/update.rs`: staging an update targets `install-update.sh`/systemd (`csr-update@.path`) – `stage` is refused on Windows ("download the new zip and run install.ps1 again"); `/api/update` reports `auto_install: false` and the dashboard shows that text instead of the Update button. The "new version available" notice stays.
- `/var/lib/csr`: not hard-coded in `runner/src`; it appears only in `config/` and the installer (`--data-root`), which stay unchanged.
- No `nix`, `users`, `uid` or SIGHUP use exists in the runner.
