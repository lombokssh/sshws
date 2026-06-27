#!/bin/bash

# ponytail: native github aliases cover this. Skipped API parsing/grep.
curl -fsSL -o /usr/local/bin/sshws https://github.com/lombokssh/sshws/releases/latest/download/sshws
chmod +x /usr/local/bin/sshws

groupadd -f tunnelusers

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

systemctl daemon-reload
systemctl enable --now sshws
systemctl restart sshd