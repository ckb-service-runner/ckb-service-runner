# Install and run CSR on Linux (quick start)

This guide installs **CKB Service Runner (CSR)** on an **Ubuntu/Debian x86_64** machine and runs it as a
background service. It takes about 15 minutes.

CSR runs **beside an existing CKB full node**. It does **not** install or synchronise the node for you.
For every detail (upgrades, uninstall, remote access) see the [full installation guide](install.md).

## 1. Check the prerequisites

You need:

- A Linux x86_64 machine with `systemd` and `sudo` access.
- A **fully synced CKB full node** (not a light client) on the **same machine**, RPC on `http://127.0.0.1:8114`.
- The `Indexer` and `Net` RPC modules enabled in the node's `ckb.toml`. Under `[rpc]`, the `modules` list must
  contain both, for example:

  ```toml
  modules = ["Net", "Pool", "Miner", "Chain", "Stats", "Subscription", "Experiment", "Debug", "Indexer"]
  ```

  Restart the CKB node after editing `ckb.toml`.

Install the tools used below:

```bash
sudo apt update
sudo apt install -y curl ca-certificates acl
```

> **Important:** CSR only accepts a loopback RPC address. Never expose the node's RPC port to the internet.

## 2. Download, verify and install CSR

These commands fetch the latest release, check its checksum and install the `csr` binary:

```bash
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

**Stay in this folder for steps 3 to 6**: it holds the example configuration and the service files.

## 3. Create the CSR user and configuration

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
enabled = false
```

`ping_mode = "outbound"` works behind a home router or firewall: CSR contacts the monitor itself, so you do not
have to open a port. (If your machine accepts inbound connections on TCP 8131 you can use `"inbound"` instead.)
The runtime stays off until the installation works.

## 4. Prove your node's identity (recommended)

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

## 5. Check the connection

```bash
sudo -u csr csr --data-root /var/lib/csr config validate
sudo -u csr csr --data-root /var/lib/csr detect
sudo -u csr csr --data-root /var/lib/csr check
```

`config validate` prints `config ok`. `check` must report the node as synced and, if you did step 4,
`node identity: proven`. If not, fix the node, the RPC address or the RPC modules first.

## 6. Start CSR and enable automatic updates

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
their signature (you start the update from the dashboard, see step 7).

**Is your CKB node's data under `/home`?** The service cannot see `/home` by default, so the identity proof of
step 4 would fail. Allow just the key file:

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

## 7. Open the dashboard

On the CSR machine open **http://127.0.0.1:8130**.

If CSR runs on a remote server, open an SSH tunnel **from your own computer** and keep it open:

```bash
ssh -L 8130:127.0.0.1:8130 YOUR_USER@YOUR_SERVER
```

Then open **http://127.0.0.1:8130** on your computer.

Set a dashboard password. Without one the dashboard is read-only: no updates, wallet, registration or
settings changes.

```bash
sudo -u csr csr --data-root /var/lib/csr set-password
sudo systemctl restart csr
```

Do not expose the dashboard to the internet: it speaks plain HTTP.

## 8. Optional: register and run service jobs (testnet)

Ranking and the dashboard work without this step. To earn tips from service jobs, the runner must be
registered:

1. Get testnet CKB from the [Nervos faucet](https://faucet.nervos.org).
2. Send about **700 CKB** to the runner address shown on the dashboard page **Wallet & registration**.
3. Make a **Nervos DAO deposit of at least 10,000 CKB** from your own wallet (for example Neuron). It stays
   yours; the registration only refers to it.
4. On **Wallet & registration**, enter the deposit, sign the shown text in Neuron ("Sign message"), paste the
   signature and register.
5. Set `[runtime] enabled = true` in `/var/lib/csr/csr.toml` (or switch the runtime on in the dashboard),
   approve only services you trust, and restart CSR.

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
