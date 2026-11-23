#!/bin/sh
#
# One-click build/install for nm-freebsd on FreeBSD.
#
#   sudo scripts/install.sh
#   sudo scripts/install.sh --no-start --no-smoke
#   sudo scripts/install.sh --uninstall --yes
#   scripts/install.sh --destdir /tmp/stage --no-start --no-smoke
#
set -u

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
prefix=/usr/local
destdir=
plugin=auto
build=1
install_deps=0
enable=1
start=1
smoke=1
uninstall=0
purge=0
dry=0
yes=0

usage() {
    cat <<'EOF'
usage: scripts/install.sh [options]

  --prefix DIR        installation prefix (default /usr/local)
  --destdir DIR       stage files under DIR (no service changes)
  --with-plugin       force building/installing the XFCE plugin
  --no-plugin         do not build/install the XFCE plugin
  --no-build          install existing target/release artifacts
  --install-deps      run pkg install for missing build dependencies (root)
  --no-enable         do not sysrc nm_freebsd_enable=YES
  --no-start          do not start/restart the rc service
  --no-smoke          do not run scripts/smoke-test.sh after starting
  --uninstall         stop and remove installed files
  --purge             with --uninstall, also remove /var/db/nm-freebsd
  --dry-run           print what would be done
  --yes               assume yes for destructive operations
  -h, --help          this help
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) shift; prefix=$1 ;;
        --destdir) shift; destdir=$1 ;;
        --with-plugin) plugin=1 ;;
        --no-plugin) plugin=0 ;;
        --no-build) build=0 ;;
        --install-deps) install_deps=1 ;;
        --no-enable) enable=0 ;;
        --no-start) start=0 ;;
        --no-smoke) smoke=0 ;;
        --uninstall) uninstall=1 ;;
        --purge) purge=1 ;;
        --dry-run) dry=1 ;;
        --yes) yes=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage; exit 2 ;;
    esac
    shift
done

if [ -n "$destdir" ]; then
    destdir=$(printf '%s' "$destdir" | sed 's|/$||')
fi
case "$prefix" in
    /*) ;;
    *) echo "install: --prefix must be an absolute path" >&2; exit 2 ;;
esac

if [ "$plugin" = "auto" ]; then
    if command -v pkg-config >/dev/null 2>&1 && \
        pkg-config --exists gtk+-3.0 libxfce4panel-2.0 2>/dev/null; then
        plugin=1
    else
        plugin=0
    fi
fi

if [ "$(id -u)" -ne 0 ] && [ -z "$destdir" ] && [ "$dry" -eq 0 ]; then
    echo "install: must be run as root (or use --destdir for a staged install)" >&2
    exit 1
fi

install_user=${SUDO_USER:-${DOAS_USER:-}}
if [ -z "$destdir" ] && [ "$uninstall" -eq 0 ] && [ -n "$install_user" ] && [ "$install_user" != "root" ]; then
    if ! id -Gn "$install_user" 2>/dev/null | tr ' ' '\n' | grep -qxE 'wheel|network'; then
        echo "install: WARNING: $install_user is not in the wheel or network group." >&2
        echo "install: the desktop will be read-only; grant control with:" >&2
        echo "  pw groupmod network -m $install_user" >&2
        echo "install: then log out and back in." >&2
    fi
fi

if [ -z "$destdir" ] && [ "$dry" -eq 0 ] && [ "$uninstall" -eq 1 ] && [ "$yes" -eq 0 ]; then
    printf 'Uninstall nm-freebsd and remove installed system files? [y/N] '
    read answer
    case "$answer" in
        y|Y|yes|YES) ;;
        *) echo "cancelled"; exit 0 ;;
    esac
fi

target_prefix=$destdir$prefix
target_state=$destdir/var/db/nm-freebsd

run() {
    if [ "$dry" -eq 1 ]; then
        printf '+ %s\n' "$*"
    else
        "$@"
    fi
}

backup() {
    file=$1
    if [ "$dry" -eq 1 ] || [ -n "$destdir" ]; then
        return 0
    fi
    if [ -e "$file" ] && [ ! -e "$file.nm-freebsd.bak" ]; then
        cp -p "$file" "$file.nm-freebsd.bak"
    fi
}

restore() {
    file=$1
    if [ "$dry" -eq 1 ]; then
        return 0
    fi
    if [ -e "$file.nm-freebsd.bak" ]; then
        mv "$file.nm-freebsd.bak" "$file"
    fi
}

remove_file() {
    file=$1
    if [ -e "$file" ] || [ -L "$file" ]; then
        run rm -f "$file"
    fi
}

find_cargo() {
    cargo_cmd=
    if command -v cargo >/dev/null 2>&1; then
        cargo_cmd=$(command -v cargo)
    elif [ -x /usr/local/bin/cargo ]; then
        cargo_cmd=/usr/local/bin/cargo
    elif [ -x "$HOME/.cargo/bin/cargo" ]; then
        cargo_cmd=$HOME/.cargo/bin/cargo
    else
        for candidate in /home/*/.cargo/bin/cargo; do
            if [ -x "$candidate" ]; then
                cargo_cmd=$candidate
                break
            fi
        done
    fi
    if [ -n "$cargo_cmd" ]; then
        PATH=$(dirname "$cargo_cmd"):$PATH
        export PATH
        cargo_home=$(dirname "$(dirname "$cargo_cmd")")
        toolchain_home=$(dirname "$cargo_home")
        if [ -z "$(printenv CARGO_HOME 2>/dev/null)" ] && [ -d "$cargo_home" ]; then
            CARGO_HOME=$cargo_home
            export CARGO_HOME
        fi
        if [ -z "$(printenv RUSTUP_HOME 2>/dev/null)" ] && [ -d "$toolchain_home/.rustup" ]; then
            RUSTUP_HOME=$toolchain_home/.rustup
            export RUSTUP_HOME
        fi
    fi
}

if [ "$uninstall" -eq 1 ]; then
    echo "nm-freebsd uninstall (prefix $prefix)"
    if [ -z "$destdir" ] && command -v service >/dev/null 2>&1; then
        run service nm_freebsd stop >/dev/null 2>&1 || true
    fi
    remove_file "$target_prefix/bin/nm-freebsd-daemon"
    remove_file "$target_prefix/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf"
    remove_file "$target_prefix/share/dbus-1/system-services/org.freedesktop.NetworkManager.service"
    remove_file "$target_prefix/etc/rc.d/nm_freebsd"
    remove_file "$target_prefix/lib/xfce4/panel/plugins/libnm_freebsd_xfce.so"
    remove_file "$target_prefix/share/xfce4/panel/plugins/nm_freebsd_xfce.desktop"
    if [ -z "$destdir" ]; then
        restore "$target_prefix/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf"
        restore "$target_prefix/share/dbus-1/system-services/org.freedesktop.NetworkManager.service"
        restore "$target_prefix/etc/rc.d/nm_freebsd"
        if command -v sysrc >/dev/null 2>&1; then
            run sysrc -x nm_freebsd_enable >/dev/null 2>&1 || true
        fi
    fi
    if [ "$purge" -eq 1 ]; then
        run rm -rf "$target_state"
    fi
    echo "uninstall: OK"
    exit 0
fi

find_cargo
if [ "$build" -eq 1 ] && [ -z "$cargo_cmd" ]; then
    echo "install: cargo not found; install Rust or pass --no-build" >&2
    exit 1
fi

if [ "$install_deps" -eq 1 ]; then
    if [ "$(id -u)" -ne 0 ]; then
        echo "install: --install-deps requires root" >&2
        exit 1
    fi
    pkg_cmd=/usr/sbin/pkg
    if [ -x /usr/local/sbin/pkg ]; then
        pkg_cmd=/usr/local/sbin/pkg
    fi
    if [ "$plugin" -eq 1 ]; then
        packages="rust pkgconf dbus glib gtk3 xfce4-panel"
    else
        packages="rust pkgconf dbus glib"
    fi
    echo "installing packages: $packages"
    run "$pkg_cmd" install -y $packages
fi

echo "checking dependencies..."
if [ "$plugin" -eq 1 ]; then
    if ! "$root/scripts/check-deps.sh" --with-plugin; then
        echo "install: dependency check failed" >&2
        exit 1
    fi
else
    if ! "$root/scripts/check-deps.sh"; then
        echo "install: dependency check failed" >&2
        exit 1
    fi
fi

if [ "$build" -eq 1 ]; then
    echo "building release daemon..."
    if ! run "$cargo_cmd" build --release --locked -p nm-freebsd-daemon; then
        echo "install: daemon build failed" >&2
        exit 1
    fi
    if [ "$plugin" -eq 1 ]; then
        echo "building release XFCE plugin..."
        if ! run "$cargo_cmd" build --release --locked -p nm-xfce-plugin; then
            echo "install: plugin build failed" >&2
            exit 1
        fi
    fi
fi

daemon_bin=$root/target/release/nm-freebsd-daemon
if [ ! -x "$daemon_bin" ]; then
    echo "install: missing $daemon_bin" >&2
    exit 1
fi
plugin_so=$root/target/release/libnm_freebsd_xfce.so
plugin_desktop=
if [ "$plugin" -eq 1 ]; then
    if [ ! -f "$plugin_so" ]; then
        echo "install: missing $plugin_so" >&2
        exit 1
    fi
    plugin_desktop=$(find "$root/target/release/build" -name nm_freebsd_xfce.desktop -print -quit 2>/dev/null)
    if [ -z "$plugin_desktop" ] || [ ! -f "$plugin_desktop" ]; then
        echo "install: plugin desktop file not found (build the plugin first)" >&2
        exit 1
    fi
fi

echo "installing to $target_prefix"
run install -d -m 755 "$target_prefix/bin"
run install -d -m 755 "$target_prefix/etc/dbus-1/system.d"
run install -d -m 755 "$target_prefix/share/dbus-1/system-services"
run install -d -m 755 "$target_prefix/etc/rc.d"
run install -d -m 700 "$target_state"

backup "$target_prefix/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf"
backup "$target_prefix/share/dbus-1/system-services/org.freedesktop.NetworkManager.service"
backup "$target_prefix/etc/rc.d/nm_freebsd"

run install -m 755 "$daemon_bin" "$target_prefix/bin/nm-freebsd-daemon"
run install -m 644 "$root/packaging/freebsd/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf" \
    "$target_prefix/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf"

tmp_service=$(mktemp)
sed "s|%%PREFIX%%|$prefix|g" \
    "$root/packaging/freebsd/etc/dbus-1/system-services/org.freedesktop.NetworkManager.service.in" \
    > "$tmp_service"
run install -m 644 "$tmp_service" \
    "$target_prefix/share/dbus-1/system-services/org.freedesktop.NetworkManager.service"
rm -f "$tmp_service"

tmp_rc=$(mktemp)
sed "s|%%PREFIX%%|$prefix|g" "$root/packaging/freebsd/etc/rc.d/nm_freebsd" > "$tmp_rc"
run install -m 755 "$tmp_rc" "$target_prefix/etc/rc.d/nm_freebsd"
rm -f "$tmp_rc"

if [ "$plugin" -eq 1 ]; then
    run install -d -m 755 "$target_prefix/lib/xfce4/panel/plugins"
    run install -d -m 755 "$target_prefix/share/xfce4/panel/plugins"
    run install -m 755 "$plugin_so" \
        "$target_prefix/lib/xfce4/panel/plugins/libnm_freebsd_xfce.so"
    run install -m 644 "$plugin_desktop" \
        "$target_prefix/share/xfce4/panel/plugins/nm_freebsd_xfce.desktop"
fi

if [ -z "$destdir" ] && [ "$dry" -eq 0 ]; then
    if [ "$enable" -eq 1 ] && command -v sysrc >/dev/null 2>&1; then
        sysrc nm_freebsd_enable=YES >/dev/null
    fi
    if [ "$start" -eq 1 ] && command -v service >/dev/null 2>&1; then
        # D-Bus has no reload command in its FreeBSD rc script, but the bus
        # driver exposes ReloadConfig, which re-reads system.d policy and
        # service directories without disconnecting desktop clients.
        if command -v dbus-send >/dev/null 2>&1; then
            dbus-send --system --type=method_call --dest=org.freedesktop.DBus \
                /org/freedesktop/DBus org.freedesktop.DBus.ReloadConfig \
                >/dev/null 2>&1 || true
        fi
        if ! service nm_freebsd restart >/dev/null 2>&1; then
            service nm_freebsd start >/dev/null 2>&1 || true
        fi
    fi
fi

echo "install: OK"
echo "  daemon: $target_prefix/bin/nm-freebsd-daemon"
if [ "$plugin" -eq 1 ]; then
    echo "  plugin: $target_prefix/lib/xfce4/panel/plugins/libnm_freebsd_xfce.so"
fi
if [ -z "$destdir" ]; then
    echo "  enable at boot: sysrc nm_freebsd_enable=YES"
    echo "  start:          service nm_freebsd start"
    echo "  logs:           /var/log/nm_freebsd.log"
fi

if [ "$smoke" -eq 1 ] && [ "$start" -eq 1 ] && [ -z "$destdir" ]; then
    if [ "$plugin" -eq 1 ]; then
        if ! "$root/scripts/smoke-test.sh" --plugin --prefix "$prefix"; then
            echo "install: smoke test failed" >&2
            echo "If this is the first install, apply the new D-Bus policy with:" >&2
            echo "  service dbus restart" >&2
            exit 1
        fi
    else
        if ! "$root/scripts/smoke-test.sh"; then
            echo "install: smoke test failed" >&2
            echo "If this is the first install, apply the new D-Bus policy with:" >&2
            echo "  service dbus restart" >&2
            exit 1
        fi
    fi
elif [ "$smoke" -eq 1 ]; then
    echo "install: smoke test skipped because the service was not started"
fi
