#!/bin/sh
#
# Read-only smoke test for a running nm-freebsd service.
# Usage: scripts/smoke-test.sh [--scan] [--plugin] [--prefix DIR] [--bus-address ADDR]
#
set -u

scan=0
plugin=0
prefix=/usr/local
bus_address=
while [ $# -gt 0 ]; do
    arg=$1
    case "$arg" in
        --scan) scan=1; shift ;;
        --plugin) plugin=1; shift ;;
        --prefix)
            if [ $# -lt 2 ]; then echo "--prefix needs an argument" >&2; exit 2; fi
            prefix=$2
            shift 2
            ;;
        --bus-address)
            if [ $# -lt 2 ]; then echo "--bus-address needs an argument" >&2; exit 2; fi
            bus_address=$2
            shift 2
            ;;
        -h|--help)
            echo "usage: $0 [--scan] [--plugin] [--prefix DIR] [--bus-address ADDR]"
            exit 0
            ;;
        *)
            echo "unknown option: $arg" >&2
            exit 2
            ;;
    esac
done

if [ -n "$bus_address" ]; then
    DBUS_SYSTEM_BUS_ADDRESS=$bus_address
    export DBUS_SYSTEM_BUS_ADDRESS
fi

if ! command -v gdbus >/dev/null 2>&1; then
    echo "smoke: gdbus is required (pkg install glib)" >&2
    exit 1
fi

nm=org.freedesktop.NetworkManager
nm_path=/org/freedesktop/NetworkManager
failures=0
warnings=0

pass() {
    printf '  [pass] %s\n' "$1"
}
fail() {
    printf '  [FAIL] %s\n' "$1"
    failures=$((failures + 1))
}
warn() {
    printf '  [warn] %s\n' "$1"
    warnings=$((warnings + 1))
}

call() {
    path=$1
    method=$2
    shift 2
    gdbus call --system --dest "$nm" --object-path "$path" --method "$method" "$@"
}

get_prop() {
    path=$1
    iface=$2
    prop=$3
    gdbus call --system --dest "$nm" --object-path "$path" \
        --method org.freedesktop.DBus.Properties.Get "$iface" "$prop"
}

uint_of() {
    value=$(printf '%s' "$1" | sed 's/.* //' | tr -dc '0-9')
    value=$(printf '%s' "$value" | sed 's/^0*//')
    if [ -z "$value" ]; then
        value=0
    fi
    printf '%s' "$value"
}

echo "nm-freebsd smoke test"

if gdbus introspect --system --dest "$nm" --object-path "$nm_path" >/dev/null 2>&1; then
    pass "manager object is reachable"
else
    fail "cannot reach $nm on the system bus"
    echo "smoke: FAILED"
    exit 1
fi

version=$(get_prop "$nm_path" "$nm" Version 2>/dev/null)
if printf '%s' "$version" | grep -q '1.46.0'; then
    pass "Version 1.46.0"
else
    fail "unexpected Version: $version"
fi

state_out=$(get_prop "$nm_path" "$nm" State 2>/dev/null)
state=$(uint_of "$state_out")
case "$state" in
    10|20|30|40|50|60) pass "Manager.State = $state" ;;
    *) fail "unexpected Manager.State: $state_out" ;;
esac

devices_out=$(call "$nm_path" "$nm.GetDevices" 2>/dev/null)
devices=$(printf '%s\n' "$devices_out" | grep -o '/org/freedesktop/NetworkManager/Devices/[0-9]*' | sort -u)
device_count=$(printf '%s\n' "$devices" | grep -c '/Devices/' 2>/dev/null || true)
if [ "$device_count" -gt 0 ]; then
    pass "$device_count device object(s)"
else
    fail "no device objects"
fi

wifi=
for dev in $devices; do
    dtype=$(uint_of "$(get_prop "$dev" "org.freedesktop.NetworkManager.Device" DeviceType 2>/dev/null)")
    dstate=$(uint_of "$(get_prop "$dev" "org.freedesktop.NetworkManager.Device" State 2>/dev/null)")
    iface_out=$(get_prop "$dev" "org.freedesktop.NetworkManager.Device" Interface 2>/dev/null)
    iface=$(printf '%s' "$iface_out" | grep -o "'[^']*'" | head -1 | tr -d "'")
    printf '  [info] %s type=%s state=%s\n' "$dev" "$dtype" "$dstate"
    if [ "$dtype" = "2" ]; then
        wifi=$dev
        pass "Wi-Fi device $iface"
    fi
done

if [ -z "$wifi" ]; then
    warn "no Wi-Fi device exposed; skipping AP checks"
else
    ap_count=0
    if [ "$scan" -eq 1 ]; then
        if call "$wifi" org.freedesktop.NetworkManager.Device.Wireless.RequestScan '{}' >/dev/null 2>&1; then
            pass "RequestScan accepted"
        else
            fail "RequestScan failed"
        fi
        i=0
        while [ "$i" -lt 20 ]; do
            aps_out=$(call "$wifi" org.freedesktop.NetworkManager.Device.Wireless.GetAllAccessPoints 2>/dev/null)
            ap_count=$(printf '%s\n' "$aps_out" | grep -o '/org/freedesktop/NetworkManager/AccessPoints/[0-9]*' | wc -l | tr -d ' ')
            if [ "$ap_count" -gt 0 ]; then
                break
            fi
            sleep 1
            i=$((i + 1))
        done
        if [ "$ap_count" -gt 0 ]; then
            pass "scan returned $ap_count access point(s)"
        else
            fail "scan returned no access points"
        fi
    else
        aps_out=$(call "$wifi" org.freedesktop.NetworkManager.Device.Wireless.GetAllAccessPoints 2>/dev/null)
        ap_count=$(printf '%s\n' "$aps_out" | grep -o '/org/freedesktop/NetworkManager/AccessPoints/[0-9]*' | wc -l | tr -d ' ')
        if [ "$ap_count" -gt 0 ]; then
            pass "$ap_count cached access point(s)"
        else
            warn "no cached access points (use --scan to trigger a scan)"
        fi
    fi

    if [ "$ap_count" -gt 0 ]; then
        ap=$(printf '%s\n' "$aps_out" | grep -o '/org/freedesktop/NetworkManager/AccessPoints/[0-9]*' | head -1)
        ssid=$(get_prop "$ap" "org.freedesktop.NetworkManager.AccessPoint" Ssid 2>/dev/null)
        strength=$(uint_of "$(get_prop "$ap" "org.freedesktop.NetworkManager.AccessPoint" Strength 2>/dev/null)")
        printf '  [info] first AP %s strength=%s ssid=%s\n' "$ap" "$strength" "$ssid"
    fi
fi

if call "$nm_path" "$nm.GetPermissions" 2>/dev/null | grep -q 'wifi-connect'; then
    pass "GetPermissions includes wifi-connect"
else
    fail "GetPermissions is incomplete"
fi

if [ "$plugin" -eq 1 ]; then
    plugin_so=$prefix/lib/xfce4/panel/plugins/libnm_freebsd_xfce.so
    plugin_desktop=$prefix/share/xfce4/panel/plugins/nm_freebsd_xfce.desktop
    if [ -f "$plugin_so" ]; then
        pass "XFCE plugin library installed"
        if command -v nm >/dev/null 2>&1 && nm -D "$plugin_so" 2>/dev/null | grep -q 'xfce_panel_module_init'; then
            pass "xfce_panel_module_init exported"
        else
            warn "could not verify the plugin entry point"
        fi
    else
        fail "XFCE plugin library missing: $plugin_so"
    fi
    if [ -f "$plugin_desktop" ]; then
        pass "XFCE plugin desktop file installed"
    else
        fail "XFCE plugin desktop file missing: $plugin_desktop"
    fi
fi

if [ -z "$bus_address" ] && [ -x "$prefix/etc/rc.d/nm_freebsd" ] && command -v service >/dev/null 2>&1; then
    if service nm_freebsd status >/dev/null 2>&1; then
        pass "rc service is running"
    else
        warn "rc service is not running"
    fi
fi

echo ""
if [ "$failures" -eq 0 ]; then
    echo "smoke: PASS ($warnings warning(s))"
    exit 0
fi
echo "smoke: FAILED ($failures failure(s), $warnings warning(s))"
exit 1
