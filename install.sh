#!/bin/bash

# ponytail: combined installer and updater. Run once to install, run again to update.
# Configs won't be overwritten.

echo "Installing/Updating sshws & engselbot..."

# 1. Stop services if they exist
systemctl stop sshws engselbot 2>/dev/null || true

# 2. Download binaries
curl -fsSL -o /usr/local/bin/sshws https://github.com/lombokssh/sshws/releases/latest/download/sshws
curl -fsSL -o /usr/local/bin/engselbot https://github.com/lombokssh/sshws/releases/latest/download/engselbot
chmod +x /usr/local/bin/sshws /usr/local/bin/engselbot

# 3. Setup sshws
groupadd -f tunnelusers

# Append to sshd_config only if Port 111 isn't there already (idempotency)
if ! grep -q "Port 111" /etc/ssh/sshd_config; then
cat >> /etc/ssh/sshd_config <<EOF

# ponytail: Port must be before Match block to avoid sshd crash
Port 111

Match Group tunnelusers
    AllowTcpForwarding yes
    X11Forwarding no
    PermitTunnel no
    GatewayPorts yes
    AllowAgentForwarding no
    PermitTTY no
EOF
fi

cat > /etc/systemd/system/sshws.service <<EOF
[Unit]
Description=SSH WS TLS Proxy
After=network.target

[Service]
ExecStart=/usr/local/bin/sshws
Restart=on-failure
LimitNOFILE=1000000

[Install]
WantedBy=multi-user.target
EOF

# 4. Setup engselbot
mkdir -p /etc/engselbot
if [ ! -f /etc/engselbot/.env ]; then
cat > /etc/engselbot/.env <<EOF
RUST_LOG=info
TELOXIDE_TOKEN=
ENABLE_USER_SYNC=false
GRAPHQL_API_URL=
GRAPHQL_API_KEY=
EOF
fi

cat > /etc/systemd/system/engselbot.service <<EOF
[Unit]
Description=Engsel Telegram Bot
After=network.target

[Service]
ExecStart=/usr/local/bin/engselbot
WorkingDirectory=/etc/engselbot
Restart=on-failure
LimitNOFILE=1000000

[Install]
WantedBy=multi-user.target
EOF

# 5. Reload and start
systemctl daemon-reload
systemctl enable --now sshws engselbot
systemctl restart sshws engselbot sshd
echo "Done! Run this script again anytime to update the binaries."
