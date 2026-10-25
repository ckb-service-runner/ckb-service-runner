# Installing CSR (Phase 1 release)

CSR 0.1.0 runs beside your CKB node, shows a dashboard and joins the node ranking.
Linux x86_64 only (Windows and macOS are not supported).

## Requirements

- A synced `ckb` full node (not a light client) with the `Indexer` RPC module enabled
  (`modules` in `ckb.toml` must contain `"Indexer"`), RPC on `127.0.0.1:8114`;
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

## Upgrade

```sh
sudo systemctl stop csr
sudo install -m 0755 csr /usr/local/bin/csr
sudo systemctl start csr
```

Data in `/var/lib/csr` is kept. Read `CHANGELOG.md` for config changes.

## Uninstall

```sh
sudo systemctl disable --now csr
sudo rm /etc/systemd/system/csr.service /usr/local/bin/csr
sudo userdel csr
sudo rm -r /var/lib/csr    # deletes keys and data: back up first
```

## What is active in Phase 1

- Active: dashboard, node checks, ranking (join, ping, network page) on testnet and mainnet;
  on testnet also the service runtime.
- Not active: on **mainnet** the runtime stays `inactive` (`mainnet_runtime_unlocked = false`);
  the dashboard shows the runtime toggle greyed out: "Services start on mainnet in Phase 2".
- The unit limits CSR to 256 MB and runs it at the lowest CPU and I/O priority.
