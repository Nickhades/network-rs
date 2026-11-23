#!/bin/sh
#
# Isolated desktop rendering test for the XFCE panel plugin.
#
# Starts its own Xvfb display, private session bus, private system bus, fake
# wpa_supplicant and nm-freebsd daemon, then loads the real plugin inside a real
# xfce4-panel and captures screenshots. Your :0 session is not touched.
#
# Usage: scripts/desktop-test.sh [--no-click] [--real-wpa]
#
set -u

unset DBUS_SESSION_BUS_ADDRESS
unset DBUS_SYSTEM_BUS_ADDRESS

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

outdir=$root/artifacts/desktop-test
do_click=1
real_wpa=0
for arg in "$@"; do
    case "$arg" in
        --no-click) do_click=0 ;;
        --real-wpa) real_wpa=1 ;;
        -h|--help)
            echo "usage: $0 [--no-click] [--real-wpa]"
            exit 0
            ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

for tool in Xvfb xfce4-panel xfce4-screenshooter xwininfo xdpyinfo dbus-daemon gdbus; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "desktop-test: missing required tool: $tool" >&2
        exit 1
    fi
done
if ! ifconfig wlan0 >/dev/null 2>&1; then
    echo "desktop-test: wlan0 is required for the daemon's device enumeration" >&2
    exit 1
fi
if [ "$do_click" -eq 1 ]; then
    if ! command -v cc >/dev/null 2>&1 || ! pkg-config --exists xtst x11 2>/dev/null; then
        echo "desktop-test: XTest development files are missing (pkg install libXtst)" >&2
        exit 1
    fi
fi
if [ "$real_wpa" -eq 0 ] && ! command -v python3 >/dev/null 2>&1; then
    echo "desktop-test: python3 is required for the fake wpa server" >&2
    exit 1
fi

mkdir -p "$outdir"
# The D-Bus listen address is a plain Unix path and rejects unescaped
# non-ASCII bytes. Keep all runtime state under /tmp; copy screenshots and logs
# back into the repository at the end.
run=$(mktemp -d /tmp/nm-freebsd-desktop.XXXXXX)
mkdir -p "$run/home" "$run/runtime"
chmod 700 "$run/runtime"

panel_pid=
xfconf_pid=
xvfb_pid=
daemon_pid=
fake_pid=
system_bus_pid=
session_bus_pid=
panel_log=$run/panel.log
daemon_log=$run/daemon.log

cleanup() {
    if [ -n "$panel_pid" ]; then kill "$panel_pid" >/dev/null 2>&1 || true; fi
    if [ -n "$xfconf_pid" ]; then kill "$xfconf_pid" >/dev/null 2>&1 || true; fi
    if [ -n "$daemon_pid" ]; then kill "$daemon_pid" >/dev/null 2>&1 || true; fi
    if [ -n "$fake_pid" ]; then kill "$fake_pid" >/dev/null 2>&1 || true; fi
    if [ -n "$xvfb_pid" ]; then kill "$xvfb_pid" >/dev/null 2>&1 || true; fi
    if [ -n "$system_bus_pid" ]; then kill "$system_bus_pid" >/dev/null 2>&1 || true; fi
    if [ -n "$session_bus_pid" ]; then kill "$session_bus_pid" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT INT TERM

if [ ! -x target/release/nm-freebsd-daemon ] || [ ! -f target/release/libnm_freebsd_xfce.so ]; then
    echo "desktop-test: release artifacts missing; run scripts/build-test.sh first" >&2
    exit 1
fi

desktop_file=$(find target/release/build -name nm_freebsd_xfce.desktop -print -quit 2>/dev/null)
if [ -z "$desktop_file" ] || [ ! -f "$desktop_file" ]; then
    echo "desktop-test: plugin desktop file missing; build the plugin first" >&2
    exit 1
fi

# ---- private display -------------------------------------------------------
n=111
while [ -e "/tmp/.X11-unix/X$n" ]; do
    n=$((n + 1))
done
display=:$n
echo "desktop-test: Xvfb on $display"
Xvfb "$display" -screen 0 1200x800x24 -nolisten tcp >"$run/xvfb.log" 2>&1 &
xvfb_pid=$!
i=0
while [ "$i" -lt 50 ]; do
    if xdpyinfo -display "$display" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
    i=$((i + 1))
done
if ! xdpyinfo -display "$display" >/dev/null 2>&1; then
    echo "desktop-test: Xvfb did not start" >&2
    cat "$run/xvfb.log" >&2
    exit 1
fi

# ---- private session bus (xfconf / panel) ----------------------------------
eval "$(DISPLAY=$display dbus-launch --sh-syntax)"
session_bus_pid=$DBUS_SESSION_BUS_PID
export DISPLAY=$display

# ---- private system bus + fake wpa + daemon -------------------------------
system_bus=$run/system_bus
dbus_cfg=$run/system.conf
cat > "$dbus_cfg" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path=$system_bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <allow own="*"/>
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow receive_type="method_call"/>
    <allow receive_type="signal"/>
    <allow send_type="method_call"/>
    <allow send_type="signal"/>
  </policy>
</busconfig>
EOF
dbus-daemon --config-file="$dbus_cfg" --nofork >"$run/system-bus.log" 2>&1 &
system_bus_pid=$!
i=0
while [ "$i" -lt 50 ]; do
    if [ -S "$system_bus" ]; then break; fi
    sleep 0.1
    i=$((i + 1))
done
if [ ! -S "$system_bus" ]; then
    echo "desktop-test: private system bus did not start" >&2
    cat "$run/system-bus.log" >&2
    exit 1
fi
DBUS_SYSTEM_BUS_ADDRESS=unix:path=$system_bus
export DBUS_SYSTEM_BUS_ADDRESS

if [ "$real_wpa" -eq 1 ]; then
    wpa_dir=/var/run/wpa_supplicant
else
    wpa_dir=$run/wpa
    mkdir -p "$wpa_dir"
    python3 tools/fake-wpa.py "$wpa_dir/wlan0" >"$run/fake-wpa.log" 2>&1 &
    fake_pid=$!
    i=0
    while [ "$i" -lt 50 ]; do
        if [ -S "$wpa_dir/wlan0" ]; then break; fi
        sleep 0.1
        i=$((i + 1))
    done
    if [ ! -S "$wpa_dir/wlan0" ]; then
        echo "desktop-test: fake wpa socket did not appear" >&2
        cat "$run/fake-wpa.log" >&2
        exit 1
    fi
fi

target/release/nm-freebsd-daemon \
    --backend wpa-ctrl \
    --wpa-ctrl-dir "$wpa_dir" \
    --state-dir "$run/state" \
    --wifi-interface wlan0 \
    --bus-address "$DBUS_SYSTEM_BUS_ADDRESS" \
    --desktop xfce \
    --log-level info >"$daemon_log" 2>&1 &
daemon_pid=$!
i=0
while [ "$i" -lt 80 ]; do
    if gdbus introspect --system --dest org.freedesktop.NetworkManager \
        --object-path /org/freedesktop/NetworkManager >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
    i=$((i + 1))
done
if ! gdbus introspect --system --dest org.freedesktop.NetworkManager \
    --object-path /org/freedesktop/NetworkManager >/dev/null 2>&1; then
    echo "desktop-test: NetworkManager service did not appear" >&2
    cat "$daemon_log" >&2
    exit 1
fi
sleep 4

# ---- temporary HOME and plugin install ------------------------------------
home=$run/home
export HOME=$home
export XDG_CONFIG_HOME=$home/.config
export XDG_DATA_HOME=$home/.local/share
export XDG_CACHE_HOME=$home/.cache
export XDG_RUNTIME_DIR=$run/runtime
mkdir -p "$XDG_CONFIG_HOME/xfce4/xfconf/xfce-perchannel-xml"
mkdir -p "$home/.local/lib/xfce4/panel/plugins"
mkdir -p "$XDG_DATA_HOME/xfce4/panel/plugins"
install -m 755 target/release/libnm_freebsd_xfce.so \
    "$home/.local/lib/xfce4/panel/plugins/libnm_freebsd_xfce.so"
install -m 644 "$desktop_file" \
    "$XDG_DATA_HOME/xfce4/panel/plugins/nm_freebsd_xfce.desktop"

cat > "$XDG_CONFIG_HOME/xfce4/xfconf/xfce-perchannel-xml/xfce4-panel.xml" <<'EOF'
<?xml version="1.1" encoding="UTF-8"?>

<channel name="xfce4-panel" version="1.0">
  <property name="configver" type="int" value="2"/>
  <property name="panels" type="array">
    <value type="int" value="1"/>
    <property name="panel-1" type="empty">
      <property name="position" type="string" value="p=6;x=600;y=0"/>
      <property name="length" type="double" value="100"/>
      <property name="position-locked" type="bool" value="true"/>
      <property name="size" type="uint" value="48"/>
      <property name="icon-size" type="uint" value="0"/>
      <property name="mode" type="uint" value="0"/>
      <property name="background-style" type="uint" value="0"/>
      <property name="background-rgba" type="array">
        <value type="double" value="0.12"/>
        <value type="double" value="0.12"/>
        <value type="double" value="0.12"/>
        <value type="double" value="1.0"/>
      </property>
      <property name="plugin-ids" type="array">
        <value type="int" value="1"/>
      </property>
    </property>
  </property>
  <property name="plugins" type="empty">
    <property name="plugin-1" type="string" value="nm_freebsd_xfce"/>
  </property>
</channel>
EOF

# xfconfd is normally D-Bus activated; run it explicitly so the temp config is
# read before the panel starts.
/usr/local/lib/xfce4/xfconf/xfconfd >"$run/xfconfd.log" 2>&1 &
xfconf_pid=$!
sleep 1

xdg_config_home=$XDG_CONFIG_HOME xfce4-panel -d >"$panel_log" 2>&1 &
panel_pid=$!
sleep 4
if ! kill -0 "$panel_pid" >/dev/null 2>&1; then
    echo "desktop-test: xfce4-panel exited" >&2
    cat "$panel_log" >&2
    exit 1
fi

xwininfo -display "$display" -root -tree >"$run/window-tree.txt" 2>&1
if ! grep -qi 'xfce4-panel' "$run/window-tree.txt"; then
    echo "desktop-test: panel window not found" >&2
    cat "$run/window-tree.txt" >&2
    cat "$panel_log" >&2
    exit 1
fi

shot=$run/nm-freebsd-panel.png
rm -f "$shot"
xfce4-screenshooter -f -s "$shot" --display="$display" >"$run/screenshooter.log" 2>&1 || true
if [ ! -s "$shot" ]; then
    if ! ffmpeg -hide_banner -loglevel error -f x11grab -video_size 1200x800 \
        -i "$display" -frames:v 1 -y "$shot" >"$run/ffmpeg.log" 2>&1; then
        echo "desktop-test: screenshot failed" >&2
        cat "$run/screenshooter.log" >&2
        cat "$run/ffmpeg.log" >&2
        exit 1
    fi
fi
if [ ! -s "$shot" ]; then
    echo "desktop-test: screenshot is empty" >&2
    exit 1
fi
file "$shot"

menu_shot=$run/nm-freebsd-menu.png
if [ "$do_click" -eq 1 ]; then
    cc -o "$run/xclick" -x c - $(pkg-config --cflags --libs xtst x11) >"$run/xclick-build.log" 2>&1 <<'EOF'
#include <X11/Xlib.h>
#include <X11/extensions/XTest.h>
#include <stdlib.h>
int main(int argc, char **argv) {
    if (argc < 4) return 2;
    Display *d = XOpenDisplay(argv[1]);
    if (!d) return 3;
    int x = atoi(argv[2]);
    int y = atoi(argv[3]);
    XTestFakeMotionEvent(d, -1, x, y, CurrentTime);
    XTestFakeButtonEvent(d, 1, True, CurrentTime);
    XTestFakeButtonEvent(d, 1, False, CurrentTime);
    XFlush(d);
    XCloseDisplay(d);
    return 0;
}
EOF
    if [ -x "$run/xclick" ]; then
        rm -f "$menu_shot"
        "$run/xclick" "$display" 40 24 >/dev/null 2>&1 || true
        sleep 1
        xfce4-screenshooter -f -s "$menu_shot" --display="$display" >"$run/screenshooter-menu.log" 2>&1 || true
        if [ ! -s "$menu_shot" ]; then
            rm -f "$menu_shot"
            ffmpeg -hide_banner -loglevel error -f x11grab -video_size 1200x800 \
                -i "$display" -frames:v 1 -y "$menu_shot" >"$run/ffmpeg-menu.log" 2>&1 || true
        fi
        if [ -s "$menu_shot" ]; then
            file "$menu_shot"
        else
            echo "desktop-test: menu screenshot not produced" >&2
        fi
    else
        echo "desktop-test: could not build the XTest click helper" >&2
        cat "$run/xclick-build.log" >&2
    fi
fi

mkdir -p "$outdir/logs"
cp "$shot" "$outdir/nm-freebsd-panel.png"
if [ -s "$menu_shot" ]; then
    cp "$menu_shot" "$outdir/nm-freebsd-menu.png"
fi
cp "$run"/*.log "$outdir/logs/" 2>/dev/null || true
cp "$run/window-tree.txt" "$outdir/logs/" 2>/dev/null || true

echo ""
echo "desktop-test: OK"
echo "  display:     $display"
echo "  panel shot:  $outdir/nm-freebsd-panel.png"
if [ -s "$menu_shot" ]; then
    echo "  menu shot:   $outdir/nm-freebsd-menu.png"
fi
echo "  logs:        $outdir/logs"
echo "  runtime:     $run"
echo ""
echo "panel log tail:"
tail -20 "$panel_log" 2>/dev/null || true
