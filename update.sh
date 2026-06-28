#!/bin/bash

# ponytail: blindly stop, fetch latest, and start. Skipped version checking, just overwrite.
systemctl stop sshws engselbot 2>/dev/null || true

curl -fsSL -o /usr/local/bin/sshws https://github.com/lombokssh/sshws/releases/latest/download/sshws
curl -fsSL -o /usr/local/bin/engselbot https://github.com/lombokssh/sshws/releases/latest/download/engselbot

chmod +x /usr/local/bin/sshws /usr/local/bin/engselbot

systemctl start sshws engselbot
