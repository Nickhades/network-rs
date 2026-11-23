#!/bin/sh
#
# Check build/runtime dependencies for nm-freebsd.
# Usage: scripts/check-deps.sh [--with-plugin] [--quiet]
#
set -u

with_plugin=0
quiet=0
for arg in "$@"; do
    case "$arg" in
        --with-plugin) with_plugin=1 ;;
        --quiet) quiet=1 ;;
        -h|--help)
            echo "usage: $0 [--with-plugin] [--quiet]"
            exit 0
            ;;
        *)
            echo "unknown option: $arg" >&2
            exit 2
            ;;
    esac
done

say() {
    if [ "$quiet" -eq 0 ]; then
        printf '%s\n' "$1"
    fi
}

missing=0
ok() {
    say "  [ok]      $1"
}
bad() {
    say "  [missing] $1"
    missing=1
}
warn() {
    say "  [warn]    $1"
}

say "nm-freebsd dependency check"
say "  host: $(uname -srm 2>/dev/null || echo unknown)"

if [ "$(uname -s)" != "FreeBSD" ]; then
    warn "this project targets FreeBSD; other systems may not build the netlink backend"
fi

if ! command -v cargo >/dev/null 2>&1; then
    cargo_candidate=
    if [ -x /usr/local/bin/cargo ]; then
        cargo_candidate=/usr/local/bin/cargo
    elif [ -x "$HOME/.cargo/bin/cargo" ]; then
        cargo_candidate=$HOME/.cargo/bin/cargo
    else
        for candidate in /home/*/.cargo/bin/cargo; do
            if [ -x "$candidate" ]; then
                cargo_candidate=$candidate
                break
            fi
        done
    fi
    if [ -n "$cargo_candidate" ]; then
        PATH=$(dirname "$cargo_candidate"):$PATH
        export PATH
    fi
fi

say "build tools:"
for cmd in cc cargo rustc install sed awk; do
    if command -v "$cmd" >/dev/null 2>&1; then
        ok "$cmd"
    else
        bad "$cmd"
    fi
done

if command -v rustc >/dev/null 2>&1; then
    rust_version=$(rustc --version | awk '{print $2}')
    major=$(printf '%s' "$rust_version" | cut -d. -f1)
    minor=$(printf '%s' "$rust_version" | cut -d. -f2)
    if [ "$major" -gt 1 ] || { [ "$major" -eq 1 ] && [ "$minor" -ge 95 ]; }; then
        ok "Rust $rust_version (>= 1.95)"
    else
        bad "Rust $rust_version is older than 1.95"
    fi
fi

say "runtime:"
if command -v dbus-daemon >/dev/null 2>&1; then
    ok "dbus-daemon"
else
    bad "dbus-daemon"
fi
if command -v gdbus >/dev/null 2>&1; then
    ok "gdbus (smoke test)"
else
    warn "gdbus is missing; scripts/smoke-test.sh will not work"
fi
if command -v sysrc >/dev/null 2>&1; then
    ok "sysrc"
else
    warn "sysrc is missing; install.sh cannot enable the rc service automatically"
fi
if command -v service >/dev/null 2>&1; then
    ok "service"
else
    warn "service is missing; install.sh cannot start the rc service automatically"
fi
if [ -x /usr/sbin/wpa_supplicant ]; then
    ok "/usr/sbin/wpa_supplicant"
else
    warn "wpa_supplicant not found (base system package normally provides it)"
fi
if [ -x /sbin/dhclient ]; then
    ok "/sbin/dhclient (for existing DHCP setups)"
else
    warn "dhclient not found"
fi

if [ "$with_plugin" -eq 1 ]; then
    say "XFCE plugin libraries:"
    if command -v pkg-config >/dev/null 2>&1; then
        ok "pkg-config"
    else
        bad "pkg-config"
    fi
    for module in gtk+-3.0 libxfce4panel-2.0 libxfce4util-1.0; do
        if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists "$module" 2>/dev/null; then
            version=$(pkg-config --modversion "$module" 2>/dev/null)
            ok "$module $version"
        else
            bad "$module"
        fi
    done
    if command -v xfce4-panel >/dev/null 2>&1; then
        ok "xfce4-panel"
    else
        warn "xfce4-panel is missing; the plugin will build but cannot be loaded now"
    fi
fi

if [ "$missing" -ne 0 ]; then
    say ""
    say "Install the missing pieces (as root) with:"
    if [ "$with_plugin" -eq 1 ]; then
        say "  pkg install -y rust pkgconf dbus glib gtk3 xfce4-panel"
    else
        say "  pkg install -y rust pkgconf dbus glib"
    fi
    say "or install Rust with rustup and add ~/.cargo/bin to PATH."
    exit 1
fi

say ""
say "dependency check: OK"
exit 0
