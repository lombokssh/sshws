#!/bin/bash
apt update -y
apt install build-essential -y

# Download binary release
# Ambil versi rilis terbaru dari GitHub API
LATEST_VERSION=$(curl -s https://api.github.com/repos/ryotwell/engsel.sshws/releases/latest | grep '"tag_name":' | cut -d '"' -f 4)
echo "Mengunduh versi terbaru: $LATEST_VERSION"

wget -O /usr/local/bin/engsel-sshws "https://github.com/ryotwell/engsel.sshws/releases/download/${LATEST_VERSION}/engsel-sshws"
chmod +x /usr/local/bin/engsel-sshws

# Buat file service systemd
cat > /etc/systemd/system/engsel-sshws.service <<EOF
[Unit]
Description=SSH WS TLS Proxy
Documentation=https://google.com
After=syslog.target network-online.target

[Service]
User=root
NoNewPrivileges=true
ExecStart=/usr/local/bin/engsel-sshws
Restart=on-failure
RestartPreventExitStatus=23
LimitNPROC=10000
LimitNOFILE=1000000

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable engsel-sshws.service
systemctl restart engsel-sshws.service