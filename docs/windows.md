# CSR on Windows (beta)

Windows support is a **beta**: x86_64 only, unsigned, no auto-update. Behaviour, config format, ports and the
mainnet lock are the same as on Linux. First real install: 2 Nov 2026, Windows 11 with Neuron's internal mainnet
node (0.5.2), identity proven and ranked by the mainnet monitor.

## Who it is for

- Operators who run `ckb.exe` (testnet and/or mainnet) on a Windows 10/11 PC.
- Users of Neuron with its internal **mainnet** full node.
- **Not** Neuron on testnet: there Neuron runs a **light client**, which CSR refuses (a full node is required),
  as on Linux.

## Requirements

- Windows 10/11 x86_64, PowerShell 5.1 or later, an Administrator account.
- A synced CKB full node on the same PC with its RPC on loopback (`127.0.0.1`).
- The **Net** and **Indexer** RPC modules enabled in the node's `ckb.toml` (`modules = [..., "Net", "Indexer", ...]`).
  `ckb.exe` users edit `ckb.toml` and restart the node. **Neuron's internal mainnet node has both on by default**
  (checked on a real PC), so Neuron users change nothing. If a module is missing the installer stops.

### Neuron

- Neuron on **mainnet** runs a full node (`ckb.exe`) on RPC `http://127.0.0.1:8114`. Its data lives in the folder
  chosen in Neuron (Settings, "CKB node data"), e.g. `C:\02_CKB_Blockchain`, with the node key in the standard
  place `<folder>\data\network\secret_key`. The installer reads this folder from the running `ckb.exe`
  (its `-C` argument), so no option is needed while Neuron runs.
- Neuron on **testnet** runs a **light client** on `http://127.0.0.1:9000`; the installer recognises it and stops
  with "a CKB light client answers ... but CSR needs a full node". Neuron's light clients keep keys under
  `%APPDATA%\Neuron\chains\light\...`; those are never used.
- The node may still be syncing: CSR installs anyway and ranks once the node is synced.

## Install

1. Download `csr-<version>-x86_64-windows.zip` from the GitHub release and check it (see below). Unzip it.
2. Double-click **`install.cmd`** and allow the Administrator prompt. It runs `install.ps1`, removes the
   "downloaded from the internet" mark from the unpacked files, finds your node, and at the end creates the Start
   menu folder **CSR** (one "CSR Dashboard (network)" link per instance) and opens the dashboard in your browser.
   The window stays open until you press a key, so you can read the summary.

Starting `csr.exe` itself from the zip only prints a hint to use `install.cmd`.

With options, start PowerShell **as Administrator** in the unzipped folder instead:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\install.ps1 -CkbDir C:\path\to\node
```

Options: `-Rpc <url[]>` node RPC URL(s) (loopback http, e.g. a testnet and a mainnet node),
`-CkbDir <dir[]>` folder(s) containing a `ckb.toml` (or the node's data folder), `-NoRanking` (do not join the
ranking), `-NoBrowser` (do not open the dashboard at the end).
Without options the installer looks for nodes itself: RPC `127.0.0.1:8114`, the folders of running `ckb.exe`
processes (their `-C` argument, which covers Neuron), the user profile and `%APPDATA%\Neuron`. It copies `csr.exe` to `%ProgramFiles%\csr`, creates
`%ProgramData%\csr` (restricted to SYSTEM and Administrators), writes `csr-<network>.toml`, registers one scheduled
task per network (`CSR-testnet`, `CSR-mainnet`; at startup, as SYSTEM, restarts on failure) and asks for a
dashboard password.

### SmartScreen and unsigned scripts

The zip, `csr.exe` and the scripts are **unsigned**, so SmartScreen ("Windows protected your PC") or an
execution-policy message may appear ("More info" > "Run anyway"); `install.cmd` and the command above bypass the
policy for that one PowerShell session only. Check the download first against the signed `SHA256SUMS` of the release (signed offline; see
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

Download the new zip and run its `install.cmd` (or `install.ps1`) again. It stops the tasks, replaces `csr.exe` and starts them
again; configs, keys and data are kept. There is no auto-update; the dashboard only shows that a new version exists.

## Uninstall

Double-click `C:\Program Files\csr\uninstall.cmd` (the installer keeps a copy there), or run
`& "$env:ProgramFiles\csr\uninstall.ps1"` in an Administrator PowerShell. It removes the tasks, the firewall rules,
the Start menu folder CSR and the program folder. From a command prompt, `uninstall.cmd -RemoveData` also deletes
the data. Data in `%ProgramData%\csr` (configs, keys, wallet, logs) is kept unless you pass `-RemoveData` (you must then type DELETE).

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
