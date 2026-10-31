# Install and run CSR on Linux (quick start)

This guide installs **CKB Service Runner (CSR)** on an **Ubuntu/Debian x86_64** machine and runs it as a
background service. With the install script it takes about 5 minutes.

CSR runs **beside an existing CKB full node**. It does **not** install or synchronise the node for you.
For every detail (upgrades, uninstall, remote access) see the [full installation guide](install.md).

## 1. Check the prerequisites

You need:

- A Linux x86_64 machine with `systemd` and `sudo` access.
- A **fully synced CKB full node** (not a light client) on the **same machine**, RPC on `http://127.0.0.1:8114`.
  CSR cannot use a node on another computer: install CSR on the node's machine and open the dashboard from
  elsewhere through an SSH tunnel (step 3).
- The `Indexer` and `Net` RPC modules enabled in the node's `ckb.toml`. Under `[rpc]`, the `modules` list must
  contain both, for example:

  ```toml
  modules = ["Net", "Pool", "Miner", "Chain", "Stats", "Subscription", "Experiment", "Debug", "Indexer"]
  ```

  Restart the CKB node after editing `ckb.toml`.

> **Important:** CSR only accepts a loopback RPC address. Never expose the node's RPC port to the internet.

## 2. Install CSR

Download the install script, have a look at it, and run it:

```bash
sudo apt install -y curl            # if curl is missing (Ubuntu Desktop)
curl -fLO https://raw.githubusercontent.com/ckb-service-runner/ckb-service-runner/main/scripts/install.sh
less install.sh          # optional: read what it does (q to quit)
sudo bash install.sh
```

The script checks your node, downloads the latest release and verifies its checksum, creates the `csr` user and
`/var/lib/csr/csr.toml` (testnet, `ping_mode = "outbound"`, which works behind a home router, and the service runtime
on: it runs only the services you approve), finds your node's
P2P key and proves the node identity, starts CSR as a service with automatic updates, and asks for a dashboard
password. It never changes your CKB node or its `ckb.toml`.

At the end it prints the dashboard address and whether the node identity is `proven`. If something is
missing (node not running, `Indexer` module off) it stops and tells you what to fix: fix it and run it again.
Running it again later is safe: it upgrades CSR and keeps your configuration and keys.

Options (`sudo bash install.sh --help`):

| Option | Use |
|---|---|
| `--key PATH` | your node's key, if the script does not find it (`<ckb data dir>/network/secret_key`) |
| `--rpc URL` | the node's RPC if it is not `http://127.0.0.1:8114` |
| `--inbound` | answer the monitor on TCP 8131 instead of `outbound` (port must be reachable) |
| `--no-runtime` | install with the service runtime off (`[runtime] enabled = false`) |
| `--no-identity`, `--no-password`, `-y` | skip the identity proof, the password, or all questions |

Prefer to type each command yourself? Follow the [manual installation](#manual-installation) below instead.

## 3. Open the dashboard

On the CSR machine open **http://127.0.0.1:8130**.

If CSR runs on a remote server, open an SSH tunnel **from your own computer** and keep it open:

```bash
ssh -L 8130:127.0.0.1:8130 YOUR_USER@YOUR_SERVER
```

Then open **http://127.0.0.1:8130** on your computer.

Log in with the dashboard password you chose. Without a password the dashboard is read-only (no updates,
wallet, registration or settings changes). To set or change it:

```bash
sudo -u csr csr --data-root /var/lib/csr set-password
sudo systemctl restart csr
```

Do not expose the dashboard to the internet: it speaks plain HTTP.

## 4. Optional: register and run service jobs (testnet)

Ranking and the dashboard work without this step. To earn tips from service jobs, the runner must be
registered:

1. Get testnet CKB from the [Nervos faucet](https://faucet.nervos.org).
2. Send about **700 CKB** to the runner address shown on the dashboard page **Wallet & registration**.
3. Make a **Nervos DAO deposit of at least 10,000 CKB** from your own wallet (for example Neuron). It stays
   yours; the registration only refers to it.
4. On **Wallet & registration**, enter the deposit, sign the shown text in Neuron ("Sign message"), paste the
   signature and register.
5. On the **Services** page, approve only services you trust. The runtime is already on after the install
   script (check the Runtime card on the Overview); nothing runs until you approve a service.

Details: [Runner wallet, stake and registration](../README.md#runner-wallet-stake-and-registration).

## Useful commands

```bash
systemctl status csr --no-pager                    # status
sudo systemctl restart csr                         # restart
sudo journalctl -u csr -n 100 --no-pager           # recent log lines
sudo -u csr csr --data-root /var/lib/csr check     # node check
sudo -u csr csr --data-root /var/lib/csr update check   # is a newer release available?
```

## Notes

- **Updates:** the dashboard shows a notice when a release is available; click **Update**.
- **Backups:** `/var/lib/csr` holds the runner's keys. Back it up encrypted and keep it private.
- **Mainnet:** this guide uses testnet. Mainnet needs a one-time `--confirm-mainnet` start and a different
  update path; read the [full installation guide](install.md) first.
- **Uninstall:** see [Uninstall](install.md#uninstall).
- Problems or questions: [open an issue](https://github.com/ckb-service-runner/ckb-service-runner/issues).

## Manual installation

The same as the install script, step by step.

### M1. Download, verify and install CSR

Install the tools, then fetch the latest release, check its checksum and install the `csr` binary:

```bash
sudo apt update && sudo apt install -y curl ca-certificates acl
mkdir -p ~/csr-install && cd ~/csr-install
BASE=https://github.com/ckb-service-runner/ckb-service-runner/releases/latest/download
curl -fLO "$BASE/SHA256SUMS"
TARBALL=$(awk '/linux-x86_64\.tar\.gz$/ {print $2}' SHA256SUMS)
curl -fLO "$BASE/$TARBALL"
sha256sum -c --ignore-missing SHA256SUMS
```

The last command must print `csr-<version>-linux-x86_64.tar.gz: OK`. If it does not, stop and download again.

```bash
tar xzf "$TARBALL"
cd "${TARBALL%.tar.gz}"
sudo install -m 0755 csr /usr/local/bin/csr
csr version
```

**Stay in this folder for steps M2 to M5**: it holds the example configuration and the service files.

### M2. Create the CSR user and configuration

```bash
sudo useradd --system --home-dir /var/lib/csr --shell /usr/sbin/nologin csr
sudo install -d -o csr -g csr -m 0750 /var/lib/csr
sudo install -m 0640 -o csr -g csr csr.example.toml /var/lib/csr/csr.toml
sudo nano /var/lib/csr/csr.toml
```

For a first **testnet** run, check these values (keep everything else from the example file):

```toml
network = "testnet"

[node]
rpc_url = "http://127.0.0.1:8114"

[dashboard]
bind = "127.0.0.1:8130"

[ranking]
join = true
ping_mode = "outbound"     # the example file says "inbound": change it

[runtime]
enabled = true             # runs only services you approve
```

`ping_mode = "outbound"` works behind a home router or firewall: CSR contacts the monitor itself, so you do not
have to open a port. (If your machine accepts inbound connections on TCP 8131 you can use `"inbound"` instead.)
With the runtime on, CSR still runs nothing until you approve a service on the Services page.

### M3. Prove your node's identity (recommended)

The CSR monitor ranks a node only when CSR proves that it runs beside that node. The proof is signed with the
node's P2P key, which CSR only reads.

Find the key file (it lives in the node's data directory):

```bash
sudo find / -path '*/data/network/secret_key' -not -path '/proc/*' 2>/dev/null
```

Put the path you found in `KEY` and give the `csr` user read access to it, without changing the node:

```bash
KEY=/path/to/ckb/data/network/secret_key      # <- the path printed above
sudo setfacl -m u:csr:r "$KEY"
d=$(dirname "$KEY"); while [ "$d" != "/" ]; do sudo setfacl -m u:csr:x "$d"; d=$(dirname "$d"); done
```

Then add it to `/var/lib/csr/csr.toml` under `[node]`:

```toml
p2p_key_file = "/path/to/ckb/data/network/secret_key"
```

Never make this key readable by all users: CSR refuses such a file.

### M4. Check the connection

```bash
sudo -u csr csr --data-root /var/lib/csr config validate
sudo -u csr csr --data-root /var/lib/csr detect
sudo -u csr csr --data-root /var/lib/csr check
```

`config validate` prints `config ok`. `check` must report the node as synced and, if you did step M3,
`node identity: proven`. If not, fix the node, the RPC address or the RPC modules first.

### M5. Start CSR and enable automatic updates

Still in the extracted folder:

```bash
sudo install -m 0644 csr.service /etc/systemd/system/csr.service
sudo install -d -m 0755 /usr/local/lib/csr
sudo install -m 0755 -o root -g root install-update.sh /usr/local/lib/csr/install-update.sh
sudo install -m 0644 csr-update.path csr-update.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now csr csr-update.path
systemctl status csr --no-pager
```

CSR now starts with the machine. The small `csr-update` helper installs new releases after CSR has checked
their signature (you start the update from the dashboard).

**Is your CKB node's data under `/home`?** The service cannot see `/home` by default, so the identity proof of
step M3 would fail. Allow just the key file:

```bash
sudo systemctl edit csr
```

Paste these lines, using your key path, then save:

```ini
[Service]
ProtectHome=tmpfs
BindReadOnlyPaths=/path/to/ckb/data/network/secret_key
```

```bash
sudo systemctl restart csr
```

Follow the log with `sudo journalctl -u csr -f` (Ctrl-C to leave).

Finally set a dashboard password:

```bash
sudo -u csr csr --data-root /var/lib/csr set-password
sudo systemctl restart csr
```
