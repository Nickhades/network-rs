#!/bin/sh
#
# Golden D-Bus signal sequence using dbus-monitor against a private bus and a
# fake wpa_supplicant. It never scans or changes the real wlan0.
#
set -u

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

for tool in dbus-daemon dbus-monitor gdbus python3 ifconfig; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "signal-test: missing $tool" >&2
        exit 1
    fi
done
if ! python3 -c 'import dbus' >/dev/null 2>&1; then
    echo "signal-test: python3 dbus module is required" >&2
    exit 1
fi
if ! ifconfig wlan0 >/dev/null 2>&1; then
    echo "signal-test: wlan0 is required for device enumeration" >&2
    exit 1
fi
daemon_bin=$root/target/release/nm-freebsd-daemon
if [ ! -x "$daemon_bin" ]; then
    echo "signal-test: build the release daemon first" >&2
    exit 1
fi

run=$(mktemp -d /tmp/nm-signal.XXXXXX)
bus_pid=
wpa_pid=
daemon_pid=
monitor_pid=
cleanup() {
    [ -n "$monitor_pid" ] && kill "$monitor_pid" 2>/dev/null || true
    [ -n "$daemon_pid" ] && kill "$daemon_pid" 2>/dev/null || true
    [ -n "$wpa_pid" ] && kill "$wpa_pid" 2>/dev/null || true
    [ -n "$bus_pid" ] && kill "$bus_pid" 2>/dev/null || true
    rm -rf "$run"
}
trap cleanup EXIT INT TERM

bus=$run/bus
cat > "$run/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path=$bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/><allow own="*"/>
    <allow send_destination="*"/><allow receive_sender="*"/>
    <allow send_type="method_call"/><allow receive_type="method_call"/>
    <allow send_type="signal"/><allow receive_type="signal"/>
    <allow send_type="method_return"/><allow receive_type="method_return"/>
    <allow send_type="error"/><allow receive_type="error"/>
  </policy>
</busconfig>
EOF
dbus-daemon --config-file="$run/bus.conf" --nofork >"$run/bus.log" 2>&1 &
bus_pid=$!
i=0
while [ "$i" -lt 50 ]; do
    [ -S "$bus" ] && break
    sleep 0.1
    i=$((i + 1))
done
if [ ! -S "$bus" ]; then
    echo "signal-test: private bus did not start" >&2
    cat "$run/bus.log" >&2
    exit 1
fi
address=unix:path=$bus

mkdir -p "$run/wpa" "$run/state"
FAKE_WPA_ALTERNATE=1 python3 tools/fake-wpa.py "$run/wpa/wlan0" >"$run/wpa.log" 2>&1 &
wpa_pid=$!
i=0
while [ "$i" -lt 50 ]; do
    [ -S "$run/wpa/wlan0" ] && break
    sleep 0.1
    i=$((i + 1))
done
if [ ! -S "$run/wpa/wlan0" ]; then
    echo "signal-test: fake wpa socket did not start" >&2
    cat "$run/wpa.log" >&2
    exit 1
fi

"$daemon_bin" \
    --backend wpa-ctrl --wpa-ctrl-dir "$run/wpa" --state-dir "$run/state" \
    --bus-address "$address" --wifi-interface wlan0 --no-initial-scan \
    >"$run/daemon.log" 2>&1 &
daemon_pid=$!
i=0
while [ "$i" -lt 50 ]; do
    if gdbus call --address "$address" --dest org.freedesktop.NetworkManager \
        --object-path /org/freedesktop/NetworkManager \
        --method org.freedesktop.NetworkManager.GetDevices >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
    i=$((i + 1))
done
if ! gdbus call --address "$address" --dest org.freedesktop.NetworkManager \
    --object-path /org/freedesktop/NetworkManager \
    --method org.freedesktop.NetworkManager.GetDevices >/dev/null 2>&1; then
    echo "signal-test: daemon did not become ready" >&2
    cat "$run/daemon.log" >&2
    exit 1
fi

dbus-monitor --address "$address" >"$run/signals.log" 2>&1 &
monitor_pid=$!
sleep 0.5

python3 - "$address" <<'PY'
import dbus
import sys
import time

bus = dbus.bus.BusConnection(sys.argv[1])
manager = bus.get_object(
    "org.freedesktop.NetworkManager",
    "/org/freedesktop/NetworkManager",
)
paths = manager.GetDevices(dbus_interface="org.freedesktop.NetworkManager")
wifi = None
for path in paths:
    device = bus.get_object("org.freedesktop.NetworkManager", path)
    props = device.GetAll(
        "org.freedesktop.NetworkManager.Device",
        dbus_interface="org.freedesktop.DBus.Properties",
    )
    if int(props["DeviceType"]) == 2:
        wifi = device
        break
if wifi is None:
    raise SystemExit("signal-test: no Wi-Fi device")
for _ in range(2):
    wifi.RequestScan(
        {},
        dbus_interface="org.freedesktop.NetworkManager.Device.Wireless",
    )
    time.sleep(1.0)
PY

sleep 1
kill "$monitor_pid" 2>/dev/null || true
wait "$monitor_pid" 2>/dev/null || true
monitor_pid=

python3 - "$run/signals.log" <<'PY'
import sys
from pathlib import Path

log = Path(sys.argv[1]).read_text()
standard = "interface=org.freedesktop.DBus.Properties; member=PropertiesChanged"
assert standard in log, "missing standard PropertiesChanged"
assert "member=AccessPointAdded" in log, "missing AccessPointAdded"
assert "org.freedesktop.NetworkManager.AccessPoint" in log, "missing AP PropertiesChanged"
assert 'string "Strength"' in log, "Strength was not in the changed dictionary"
added = log.find("member=AccessPointAdded")
strength = log.find('string "Strength"')
assert added >= 0 and strength >= 0, "signal indices missing"
assert added < strength, "expected AccessPointAdded before the later Strength update"
print("signal-test: OK (PropertiesChanged, AccessPointAdded, Strength sequence)")
PY
