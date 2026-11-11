#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
#
# After a .deb or .rpm is removed or upgraded: forget the old unit.

set -e

if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
fi
