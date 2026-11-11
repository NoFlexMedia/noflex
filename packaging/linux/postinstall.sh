#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
#
# After a .deb or .rpm installs or upgrades: make the service's account, and
# restart the server if it was running. A new install doesn't start it.

set -e

if command -v systemd-sysusers >/dev/null 2>&1; then
  systemd-sysusers /usr/lib/sysusers.d/noflex.conf
fi
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  systemctl try-restart noflex.service
fi
if ! systemctl is-enabled --quiet noflex.service 2>/dev/null; then
  echo "Start NoFlex with: systemctl enable --now noflex, then open http://this-machine:8096"
fi
