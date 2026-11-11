#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
#
# Before a .deb or .rpm is removed, but not upgraded (`remove` for dpkg, 0 for
# rpm): stop the server. Its data in /var/lib/noflex stays.

set -e

case "$1" in
remove | 0)
  if [ -d /run/systemd/system ]; then
    systemctl disable --now noflex.service >/dev/null 2>&1 || true
  fi
  ;;
esac
