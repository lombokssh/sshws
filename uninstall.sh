#!/bin/bash

# ponytail: blindly undoes install.sh. Stops services, deletes files, purges the sshd_config block.

echo "Uninstalling sshws & engselbot..."

# 1. Stop and disable services
systemctl stop sshws engselbot 2>/dev/null || true
systemctl disable sshws engselbot 2>/dev/null || true

# 2. Remove systemd unit files
rm -f /etc/systemd/system/sshws.service
rm -f /etc/systemd/system/engselbot.service
systemctl daemon-reload

# 3. Remove binaries
rm -f /usr/local/bin/sshws /usr/local/bin/engselbot

# 4. Remove engselbot configs
rm -rf /etc/engselbot

# 5. Remove tunnelusers group
groupdel tunnelusers 2>/dev/null || true

# 6. Clean up sshd_config and restart sshd
sed -i '/# ponytail: Port must be before Match block to avoid sshd crash/,/PermitTTY no/d' /etc/ssh/sshd_config
systemctl restart sshd

echo "Clean uninstall complete!"
