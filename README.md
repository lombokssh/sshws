# engselwsssh / lombokssh

A high-performance WebSocket/TLS Proxy (`sshws`) and its companion Telegram Bot (`engselbot`) for seamless VPN tunneling. 

This repository contains the deployment scripts to automatically install, update, and manage both services on a Linux server.

## Features
- **sshws**: A fast, low-overhead WebSocket proxy running on port 80/443 (depending on your setup) mapping to SSH on port 111.
- **engselbot**: A Telegram bot written in Rust for managing user accounts, generating VPN configs (VLESS/TROJAN), and optionally syncing users to a GraphQL backend.

---

## 🚀 Quick Install

To install both `sshws` and `engselbot`, simply run the following one-liner command as `root` in your terminal. This will download the latest script and execute it immediately to set up the binaries and services:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/lombokssh/sshws/main/install.sh)
```

### What the installer does:
1. Downloads `sshws` and `engselbot` binaries to `/usr/local/bin/`.
2. Creates the `tunnelusers` group for secure SSH forwarding.
3. Modifies `/etc/ssh/sshd_config` to open Port 111 and apply a strict `Match Group` block for VPN users.
4. Creates systemd services (`sshws.service` and `engselbot.service`).
5. Generates a default configuration file for the bot at `/etc/engselbot/.env`.

---

## ⚙️ Configuration

After installation, you need to configure your Telegram Bot token. 

Open `/etc/engselbot/.env` with your favorite text editor:

```bash
nano /etc/engselbot/.env
```

Update it with your credentials:
```env
TELOXIDE_TOKEN=your_telegram_bot_token
ENABLE_USER_SYNC=false
GRAPHQL_API_URL=
GRAPHQL_API_KEY=
```
*If you are using the GraphQL syncing feature, set `ENABLE_USER_SYNC=true` and provide the URL and API key.*

After modifying the `.env` file, restart the bot:
```bash
systemctl restart engselbot
```

---

## 🔄 Updating

The simplest way to update the binaries to the latest version is to run the update one-liner, or you can just run the install script again. The scripts are idempotent and will not overwrite your existing `.env` config file.

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/lombokssh/sshws/main/update.sh)
```
*(This will stop the services, fetch the latest binaries from GitHub, and restart them.)*

---

## 🗑️ Uninstalling

If you wish to completely remove the proxy, the bot, and their configurations from your system, run the uninstall one-liner:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/lombokssh/sshws/main/uninstall.sh)
```

### What the uninstaller does:
1. Stops and disables both `sshws` and `engselbot` services.
2. Deletes the binaries and systemd unit files.
3. Removes the `/etc/engselbot` directory.
4. Deletes the `tunnelusers` group.
5. Cleans up the port and Match Group modifications in `/etc/ssh/sshd_config` and restarts the SSH daemon.

---

## 🛠 Service Management

You can manage the services manually using standard `systemctl` commands:

```bash
# Check status
systemctl status sshws
systemctl status engselbot

# View logs
journalctl -u sshws -f
journalctl -u engselbot -f
```
