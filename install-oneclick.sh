#!/bin/sh
#
# One-click installer for the already-built nm-freebsd release artifacts.
# Run as root from the repository root:
#
#   sudo ./install-oneclick.sh
#   sudo ./install-oneclick.sh --no-start
#   sudo ./install-oneclick.sh --destdir /tmp/nm-stage --no-start --no-smoke
#
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

if [ "$(id -u)" -ne 0 ]; then
    echo "install-oneclick: run as root (for example: sudo $0)" >&2
    exit 1
fi
if [ ! -x target/release/nm-freebsd-daemon ] || \
   [ ! -x target/release/nm-freebsd-cli ] || \
   [ ! -x target/release/nm-notify-dhcp ] || \
   [ ! -f target/release/libnm_freebsd_xfce.so ]; then
    echo "install-oneclick: prebuilt release artifacts are missing." >&2
    echo "  build them as the repository user first:" >&2
    echo "  cargo build --release --workspace" >&2
    exit 1
fi

exec "$root/scripts/install.sh" --no-build --with-plugin "$@"
