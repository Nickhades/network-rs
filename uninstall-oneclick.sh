#!/bin/sh
#
# Remove the files installed by install-oneclick.sh.
# Run as root from the repository root:
#
#   sudo ./uninstall-oneclick.sh
#   sudo ./uninstall-oneclick.sh --purge
#
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

if [ "$(id -u)" -ne 0 ]; then
    echo "uninstall-oneclick: run as root (for example: sudo $0)" >&2
    exit 1
fi

exec "$root/scripts/install.sh" --uninstall --yes "$@"
