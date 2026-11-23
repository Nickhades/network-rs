#!/bin/sh
#
# Compile, lint and test nm-freebsd. Produces build-test.log.
# Usage: scripts/build-test.sh [--no-plugin]
#
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

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

with_plugin=1
for arg in "$@"; do
    case "$arg" in
        --no-plugin) with_plugin=0 ;;
        -h|--help)
            echo "usage: $0 [--no-plugin]"
            exit 0
            ;;
        *)
            echo "unknown option: $arg" >&2
            exit 2
            ;;
    esac
done

if [ "$with_plugin" -eq 1 ]; then
    if ! command -v pkg-config >/dev/null 2>&1 || \
        ! pkg-config --exists gtk+-3.0 libxfce4panel-2.0 2>/dev/null; then
        echo "XFCE development libraries are missing; building daemon only."
        echo "Install gtk3 and xfce4-panel, or pass --no-plugin."
        with_plugin=0
    fi
fi

log="$root/build-test.log"
: > "$log"

step() {
    name=$1
    shift
    printf '\n== %s ==\n' "$name"
    if "$@" >>"$log" 2>&1; then
        printf 'OK\n'
    else
        printf 'FAILED: %s (see %s)\n' "$name" "$log" >&2
        tail -40 "$log" >&2
        exit 1
    fi
}

step "versions" sh -c 'cargo --version && rustc --version'

if cargo fmt --version >/dev/null 2>&1; then
    step "cargo fmt --check" cargo fmt --all --check
else
    printf '\n== cargo fmt ==\nSKIPPED (rustfmt component missing)\n'
fi

if cargo clippy --version >/dev/null 2>&1; then
    step "cargo clippy -D warnings" cargo clippy --workspace --all-targets -- -D warnings
else
    printf '\n== cargo clippy ==\nSKIPPED (clippy component missing)\n'
fi

if [ "$with_plugin" -eq 1 ]; then
    step "cargo test --workspace" cargo test --workspace
    step "cargo build --release --workspace" cargo build --release --workspace
else
    step "cargo test (daemon)" cargo test -p nm-common -p nm-freebsd-daemon
    step "cargo build --release (daemon)" cargo build --release -p nm-freebsd-daemon
fi

printf '\n== artifacts ==\n'
ls -lh target/release/nm-freebsd-daemon
if [ "$with_plugin" -eq 1 ]; then
    ls -lh target/release/libnm_freebsd_xfce.so
    find target/release/build -name nm_freebsd_xfce.desktop -print | head -1
fi

printf '\nbuild/test: OK (log: %s)\n' "$log"
