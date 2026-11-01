# CSR on Windows (beta)

Windows support is a **beta**: x86_64 only, unsigned, no auto-update. Behaviour, config format, ports and the
mainnet lock are the same as on Linux. Nothing here has been run on a real Windows PC by the authors yet.

## Who it is for

- Operators who run `ckb.exe` (testnet and/or mainnet) on a Windows 10/11 PC.
- Users of Neuron with its internal **mainnet** full node.
- **Not** Neuron on testnet: there Neuron runs a **light client**, which CSR refuses (a full node is required),
  as on Linux.

## Requirements

- Windows 10/11 x86_64, PowerShell 5.1 or later, an Administrator account.
- A synced CKB full node on the same PC with its RPC on loopback (`127.0.0.1`).
- The **Net** RPC module enabled in the node's `ckb.toml` (`modules = [..., "Net", ...]`); CSR needs it for peers.
  `ckb.exe` users edit `ckb.toml` and restart the node. **Neuron caveat:** Neuron's internal node may not expose
  `Net`, and it is unknown whether an edit to its `ckb.toml` survives a Neuron restart (see
  [open questions](open-questions.md)). If the Net module is missing the installer stops.

## Install

1. Download `csr-<version>-x86_64-windows.zip` from the GitHub release and check it (see below). Unzip it.
2. Start PowerShell **as Administrator** (right-click, "Run as administrator") in the unzipped folder.
3. Run:

   ```powershell
   Set-ExecutionPolicy -Scope Process Bypass
   .\install.ps1
   ```

Options: `-Rpc <url[]>` node RPC URL(s) (loopback http, e.g. a testnet and a mainnet node),
`-CkbDir <dir[]>` folder(s) containing a `ckb.toml`, `-NoRanking` (do not join the ranking).
Without options the installer looks for nodes itself. It copies `csr.exe` to `%ProgramFiles%\csr`, creates
`%ProgramData%\csr` (restricted to SYSTEM and Administrators), writes `csr-<network>.toml`, registers one scheduled
task per network (`CSR-testnet`, `CSR-mainnet`; at startup, as SYSTEM, restarts on failure) and asks for a
dashboard password.

### SmartScreen and unsigned scripts

The zip, `csr.exe` and the scripts are **unsigned**, so SmartScreen ("Windows protected your PC") or an
execution-policy message may appear; that is why the command above bypasses the policy for that one PowerShell
session only. Check the download first against the signed `SHA256SUMS` of the release (signed offline; see
[release.md](release.md)):

```powershell
Get-FileHash .\csr-<version>-x86_64-windows.zip -Algorithm SHA256
```

The hash must equal the zip's line in `SHA256SUMS`, and `SHA256SUMS` must verify against `SHA256SUMS.sig` with the
release public key.

## Dashboards and ports

- Testnet: `http://127.0.0.1:8130` (ping port 8131). Mainnet: `http://127.0.0.1:8140` (ping port 8141).
- Dashboards listen on this PC only. **Only the ping port** gets a Windows Firewall rule (inbound TCP, Private and
  Domain profiles); the dashboard port is never opened.
- On mainnet CSR is a node dashboard and ranking client; the Service Runtime stays locked, as on Linux.

## Sleep and NAT

The ranking measures availability. A PC that goes to sleep or hibernate, or is switched off, is unreachable and
loses availability in the ranking: set the power plan to never sleep for a steady rank. Behind a home router
(NAT) the ping port must be forwarded to the PC, or the monitor cannot reach it.

## Update

Download the new zip and run its `install.ps1` again. It stops the tasks, replaces `csr.exe` and starts them
again; configs, keys and data are kept. There is no auto-update; the dashboard only shows that a new version exists.

## Uninstall

Run `.\uninstall.ps1` from the zip in an Administrator PowerShell. It removes the tasks, the firewall rules and
the program folder. Data in `%ProgramData%\csr` (configs, keys, wallet, logs) is kept unless you pass `-RemoveData` (you must then type DELETE).

## Logs

`%ProgramData%\csr\<network>\logs\csr.log` (for example `C:\ProgramData\csr\testnet\logs\csr.log`), also shown in the
dashboard. Task state: `Get-ScheduledTask CSR-testnet` (or `CSR-mainnet`).

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
