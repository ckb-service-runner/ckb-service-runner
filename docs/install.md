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

## Testnet migration to contracts v3

Before mainnet the CSR contracts are redeployed once on testnet (contracts v3: Registry v3, `CSR-ATTEST-v2`,
the `csr-config-type` monitor config cell; `spec/registry.md`). Registrations under the old contracts do not
carry over: every operator registers **once more**. What to do:

1. **Update the runner** to the release that embeds the v3 `deployments/testnet.json` (see "Upgrade"). Until
   then the runner refuses `csr register create` with a message that the deployment is still v2; jobs of the
   old deployment keep running.
2. **Optional: set the public endpoint.** `[ranking] public_endpoint = "https://<your host>:8131"` (or
   `http://…`, at most 128 bytes, no path) is written into the registry cell so that the monitor can call your
   runner directly. Leave it empty if you do not want to publish an address.
3. **Fund the runner wallet**: a v3 registry cell needs about **420 CKB** (about 125 CKB less than v2) plus the
   61 CKB that stay in the wallet and the fee.
4. **Register again** with the same Nervos DAO deposit: `csr register consent-message --stake 0x<tx>:<index>`,
   sign the text in Neuron ("Sign message"), then `csr register create --stake 0x<tx>:<index> --consent
   0x<signature>` (or the dashboard). The runner asks the monitor for a new `CSR-ATTEST-v2` attestation, which
   is bound to this deposit and to the v3 registry. Old attestation files (`csr register attest` output of
   earlier releases) cannot be used.
5. **Remove the old registration** to get its capacity back: run `csr register remove` with the previous
   release *before* updating, or reclaim it later with the old release. The deposit is only referenced, never
   moved, so it stays where it is.

Changing the node identity (`peer_id`, network or service key) of a v3 registration always needs a new consent
of the stake owner. A deposit that is withdrawn keeps a registration earning only until its attestation expires
(at most 7 days, 8 by the on-chain cap).

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
