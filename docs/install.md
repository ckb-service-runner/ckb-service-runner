# Installing CSR (Phase 1 release)

CSR 0.1.0 runs beside your CKB node, shows a dashboard and joins the node ranking.
Linux x86_64 only (Windows and macOS are not supported).

## Requirements

- A synced `ckb` full node (not a light client) with the `Indexer` RPC module enabled
  (`modules` in `ckb.toml` must contain `"Indexer"`, and `"Net"` for the node identity check,
  see "Node identity"), RPC on `127.0.0.1:8114`;
  or Neuron running in full-node mode.
- systemd, about 50 MB of RAM and a little disk.

## Install from the tarball

```sh
sha256sum -c SHA256SUMS
tar xzf csr-0.1.0-linux-x86_64.tar.gz
cd csr-0.1.0-linux-x86_64
sudo install -m 0755 csr /usr/local/bin/csr
```

## User and data directory

```sh
sudo useradd --system --home-dir /var/lib/csr --shell /usr/sbin/nologin csr
sudo install -d -o csr -g csr -m 0750 /var/lib/csr
```

## Configuration

```sh
sudo install -m 0640 -o csr -g csr csr.example.toml /var/lib/csr/csr.toml
sudoedit /var/lib/csr/csr.toml
```

Set `network = "testnet"` or `"mainnet"` and `node.rpc_url`. Then check the setup as the `csr` user:

```sh
sudo -u csr csr --data-root /var/lib/csr detect
sudo -u csr csr --data-root /var/lib/csr check
```

`csr detect` finds the node and its type; `csr check` runs the node checks (synced, `Indexer` module,
full node, peers) and prints what is wrong.

## Start the service

```sh
sudo install -m 0644 csr.service /etc/systemd/system/csr.service
sudo systemctl daemon-reload
sudo systemctl enable --now csr
journalctl -u csr -f
```

On **mainnet** the first start needs an explicit confirmation: edit the unit
(`sudo systemctl edit --full csr`), use the commented `--confirm-mainnet` `ExecStart` line once,
start the service, then switch back to the normal line.

## Dashboard

The dashboard listens on `127.0.0.1:8130`. From your workstation use an SSH tunnel:

```sh
ssh -L 8130:127.0.0.1:8130 user@node-host
```

and open <http://127.0.0.1:8130>. To bind it to the LAN (`dashboard.bind = "0.0.0.0:8130"`) you
must set a password first:

```sh
sudo -u csr csr --data-root /var/lib/csr set-password
```

### Remote access

A dashboard bound to a non-loopback address (`0.0.0.0:8130`) speaks plain HTTP: the password and the
session cookie cross the network in clear. Use an SSH tunnel (preferred, see above), or put a TLS
reverse proxy in front that sets `X-Forwarded-Proto: https` (the session cookie then gets `Secure`).
A dashboard password is required for wallet, registration, service and settings changes; without one
these writes are refused, and only the read-only pages work.

## Stolen disk

The storage key lives in `keys/` beside the data it protects, so it does not help against a stolen disk
or a copied backup. Protect against that with full-disk encryption (for example LUKS). Do not back up
`keys/` in clear; keep any backup of it encrypted and apart from the data.

## Ping port

The ranking monitor pings your node on **8131/tcp** (`ranking.ping_bind`): open it in your firewall
and forward it if you are behind NAT. If you cannot accept inbound connections, set
`ranking.ping_mode = "outbound"` in `csr.toml` (the node connects out to the monitor instead).

## Node identity

The monitors rank a node only if the runner proves that it runs beside that node (decision D6 A). The
proof is signed with the node's **P2P secret key**: the key the CKB node uses on the peer-to-peer network,
from which its node ID (`Qm…`, `local_node_info.node_id`) is derived. CKB keeps it in
`<ckb data dir>/network/secret_key` (32 raw bytes; 64 hex characters are accepted too). Without the key
anybody could claim any well-connected node; with it, only the operator of the node can.

Set the path in `csr.toml`:

```toml
[node]
p2p_key_file = "/var/lib/ckb/data/network/secret_key"
```

The runner only **reads** the file, at start: it never writes it, never changes its mode or owner, keeps the
key in memory only and never shows it in a log, the dashboard or an API answer. It refuses a file readable
by every user ("other"), and it refuses to send a proof if the key does not derive the node ID the node
reports (wrong file); both show on the Node page and in `csr check`. Without `p2p_key_file` everything works
as before, but the monitors will not rank the node once they require the proof.

Grant the `csr` user read access without changing the node — an ACL on the file and `x` (search, not
read) on each directory above it:

```sh
sudo setfacl -m u:csr:r /var/lib/ckb/data/network/secret_key
sudo setfacl -m u:csr:x /var/lib/ckb /var/lib/ckb/data /var/lib/ckb/data/network
```

or, if the node's files belong to a group of their own, add `csr` to it (`sudo usermod -aG ckb csr`) and
make sure the file is group-readable (`chmod g+r`; not `o+r`). Restart CSR after changing
`p2p_key_file`. Check it:

```sh
sudo -u csr csr --data-root /var/lib/csr check
```

prints `node identity: proven` (other states: `off` = not configured, `mismatch` = the key belongs to
another node, `unreadable` = missing, no access, readable by everyone or not a key, `unknown` = the node does
not answer).

**Behind NAT.** Each monitor publishes the P2P address of its own CKB node (D12 A). CSR asks your node to
connect to it once at start and every 6 hours (`add_node`), so that the monitor sees your node among its
peers even if nobody can connect in. This needs the node's **Net** RPC module: `modules` under `[rpc]` in
`ckb.toml` must include `"Net"` (restart the node after the change). If it is missing, CSR logs one warning
and the Node page shows it. CSR never removes peers or calls other Net methods.

## Upgrade

### From the dashboard (signed releases)

The Overview shows a notice when a newer release exists (CSR asks GitHub once a day; only the
request itself is sent). "Check for update" asks at once; "Update" downloads the binary for this
machine, `SHA256SUMS` and `SHA256SUMS.sig` from the GitHub release, checks the signature against the
release key built into CSR and the binary's SHA-256, and stages it in
`/var/lib/csr/<network>/update/`. The same on the command line:

```sh
sudo -u csr csr --data-root /var/lib/csr update check
sudo -u csr csr --data-root /var/lib/csr update stage
```

The dashboard's Update button needs a dashboard password (`csr set-password`); without one use the
command line. Builds without a configured release key refuse every update ("no release key
configured"): upgrade manually (below).

CSR runs as the unprivileged `csr` user and cannot replace its own binary. A small root installer
does it: install it once.

```sh
sudo install -d -m 0755 /usr/local/lib/csr
sudo install -m 0755 -o root -g root install-update.sh /usr/local/lib/csr/install-update.sh
sudo install -m 0644 csr-update.path csr-update.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now csr-update.path
```

(All three files are in the release tarball; in the repository they are `scripts/install-update.sh` and `config/`.)
The shipped units watch `/var/lib/csr/testnet/update/ready.json`. For **mainnet**, or another data
root, change the path in both files: `PathExists=` in `csr-update.path` and the argument of
`ExecStart=` in `csr-update.service` must be `<data root>/<network>/update` (with `ready.json` in
the path unit). Run `sudo systemctl daemon-reload` and restart `csr-update.path` afterwards.

When `ready.json` appears, `csr-update.service` runs `install-update.sh`, which

1. copies the staged files into a root-only directory (read as the `csr` user),
2. lets the **installed** `/usr/local/bin/csr update verify` check them (signature, SHA-256, newer
   version; no network),
3. keeps the current binary as `/usr/local/bin/csr.prev`, installs the new one
   (`install -m 0755 -o root -g root`), removes the staged files and restarts `csr`;
   if the new binary's `csr --version` fails, it puts `csr.prev` back and restarts.

A failed check moves the staged files to `/var/lib/csr/<network>/update/rejected/`. Logs:
`journalctl -u csr-update`.

### Roll back

```sh
sudo install -m 0755 -o root -g root /usr/local/bin/csr.prev /usr/local/bin/csr
sudo systemctl restart csr
```

(The updater itself never installs an older version.)

### Manually

```sh
sudo systemctl stop csr
sudo install -m 0755 csr /usr/local/bin/csr
sudo systemctl start csr
```

Data in `/var/lib/csr` is kept. Read `CHANGELOG.md` for config changes.

## Uninstall

```sh
sudo systemctl disable --now csr csr-update.path
sudo rm /etc/systemd/system/csr.service /etc/systemd/system/csr-update.path \
  /etc/systemd/system/csr-update.service /usr/local/bin/csr /usr/local/bin/csr.prev
sudo rm -r /usr/local/lib/csr
sudo userdel csr
sudo rm -r /var/lib/csr    # deletes keys and data: back up first
```

## What is active in Phase 1

- Active: dashboard, node checks, ranking (join, ping, network page) on testnet and mainnet;
  on testnet also the service runtime.
- Not active: on **mainnet** the runtime stays `inactive` (`mainnet_runtime_unlocked = false`);
  the dashboard shows the runtime toggle greyed out: "Services start on mainnet in Phase 2".
- The unit limits CSR to 256 MB and runs it at the lowest CPU and I/O priority.
