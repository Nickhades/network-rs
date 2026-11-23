#!/bin/sh
#
# Install the nm-freebsd GNOME Shell extension for the current user.
#
#   scripts/install-gnome-extension.sh
#   scripts/install-gnome-extension.sh --no-enable
#
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
uuid=nm-freebsd@nickhades.github.io
source_dir=$root/gnome-extension/$uuid
dest_dir=$HOME/.local/share/gnome-shell/extensions/$uuid
enable=1

for arg in "$@"; do
    case "$arg" in
        --no-enable) enable=0 ;;
        -h|--help)
            echo "usage: $0 [--no-enable]"
            exit 0
            ;;
        *)
            echo "unknown option: $arg" >&2
            exit 2
            ;;
    esac
done

if [ ! -f "$source_dir/metadata.json" ]; then
    echo "install-gnome-extension: extension source is missing" >&2
    exit 1
fi
if ! command -v gsettings >/dev/null 2>&1; then
    echo "install-gnome-extension: gsettings is required" >&2
    exit 1
fi

install -d "$dest_dir"
cp "$source_dir/metadata.json" "$dest_dir/metadata.json"
cp "$source_dir/extension.js" "$dest_dir/extension.js"
if [ -f "$source_dir/stylesheet.css" ]; then
    cp "$source_dir/stylesheet.css" "$dest_dir/stylesheet.css"
fi

if [ "$enable" -eq 1 ]; then
    current=$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo "@as []")
    case "$current" in
    *"$uuid"*) ;;
    "@as []"|"[]"|"")
        gsettings set org.gnome.shell enabled-extensions "['$uuid']"
        ;;
    *)
        updated=$(printf '%s' "$current" | sed "s/]$/, '$uuid']/")
        gsettings set org.gnome.shell enabled-extensions "$updated"
        ;;
    esac
fi

echo "gnome-extension: installed $uuid"
echo "  path: $dest_dir"
if [ "$enable" -eq 1 ]; then
    echo "  enabled in org.gnome.shell enabled-extensions"
fi
echo "Log out and back in (or restart GNOME Shell) to load the extension."
echo "If the extension is not listed, check:"
echo "  gsettings get org.gnome.shell enabled-extensions"
