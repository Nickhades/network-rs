# Testing

**Languages:** [English](TESTING.md) | [简体中文](zh-CN/TESTING.md)

## Automated

The convenience entry points are:

```sh
scripts/check-deps.sh --with-plugin
scripts/build-test.sh
scripts/smoke-test.sh --scan
scripts/desktop-test.sh
scripts/signal-test.sh
```

`build-test.sh` runs fmt, clippy, the whole workspace test suite and a release
build, logging to `build-test.log`. The raw commands are:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

`scripts/signal-test.sh` is the literal `dbus-monitor` golden sequence:
private bus, fake wpa with alternating strength, two RequestScan calls, then
checks for standard `PropertiesChanged`, `AccessPointAdded` and a later
`Strength` update. `build-test.sh` runs it when the tools and wlan0 enumeration
are available.

The suite contains:

- `nm-common` tests: desktop detection, wpa_supplicant security flag
  parsing, SSID byte/hex/quoting helpers.
- daemon unit tests: wpa scan/status parsing, netlink message parsing
  (synthetic and live link dump), getifaddrs/ifindex consistency, profile
  persistence, keyring storage/migration and 0600 permissions, caller
  permission maps (`wheel`/`network`), DHCP backend/process lifecycle,
  netmask-to-prefix conversion, settings round-trips, UUID path-traversal
  rejection, CLI parsing.
- one plugin unit test for v4 UUID shape.
- integration test `mvp_end_to_end` (private bus, fake datagram
  wpa_supplicant, real daemon, zbus client) in
  `crates/nm-freebsd-daemon/tests/integration.rs`;
- integration test `request_scan_uses_wpa_dbus_scan_done` (mock
  `fi.w1.wpa_supplicant1` service) in
  `crates/nm-freebsd-daemon/tests/wpa_dbus_mock.rs`: proves the upper
  `RequestScan` reaches `Interface.Scan`, and a `ScanDone` signal refreshes
  NetworkManager APs from `BSSs`; see [SCAN-FLOW.md](SCAN-FLOW.md).

The integration test is self-contained:

1. starts a private `dbus-daemon` with a permissive bus configuration;
2. starts a fake wpa_supplicant as a **Unix datagram** server;
3. starts the real daemon through `daemon::start` with an explicit bus
   address and a temporary state directory;
4. connects a real zbus client and checks:
   - Wi-Fi device discovery;
   - `RequestScan` and `GetAllAccessPoints`, including SSID, strength and
     privacy flags;
   - `AddAndActivateConnection`;
   - wpa_supplicant received `ADD_NETWORK`, the SSID as hex, `PSK`,
     `SELECT_NETWORK`;
   - active connection state 2, device state 100, manager state 60;
   - `Manager.StateChanged` was emitted;
   - `Settings.ListConnections`/`GetConnection` round-trip;
   - `GetPermissions` returns all 11 permission keys and `wifi-scan = yes`
     (admin/read-only decisions are unit-tested separately);
   - an AP `Strength` change emits the standard
     `org.freedesktop.DBus.Properties.PropertiesChanged` on the AP path;
   - the Device `StateChanged` sequence contains `IP_CONFIG` (70) and ends
     at `ACTIVATED` (100);
   - `DeactivateConnection` returns the device to state 30 and the daemon to
     20;
   - a simulated `Dhcp.Notify` BOUND event publishes IP4Config with address
     10.99.0.2/24 and Gateway 10.99.0.1 (the hook data path).

It does not touch the host network. It skips gracefully if no `dbus-daemon`
binary is available.

## DHCP verification

The integration test pushes a simulated BOUND through the internal
`Dhcp.Notify` method and checks IP4Config Gateway/AddressData. The daemon unit
tests cover netmask conversion, backend selection, the disabled backend, and a
fake `dhclient` script (start, managed PID, stop). The real
`dhclient -d` plus root-owned exit hook cannot be exercised without root and
without risking the host link, so it remains a manual/live-test item.

The staged installer test checks that `/etc/dhclient-exit-hooks` receives the
guarded source line and that the notifier/hook binaries land in
`/usr/local/libexec`.

## Security policy verification

`GetPermissions` is exercised by the integration test. The D-Bus policy was
also checked on a private bus with a copy of the policy whose admin groups were
replaced by an empty group, so the test uid used the unprivileged path:

- `GetDevices` and `GetPermissions` were allowed;
- `ActivateConnection` and `Properties.Set` failed with
  `org.freedesktop.DBus.Error.AccessDenied`;
- with the real `wheel` policy the same write call succeeded.

This check needs a second uid/group database, so it is documented rather than
part of `cargo test`. The daemon-side uid check is unit-tested and covers the
case where the policy is accidentally too permissive.

## Live read-only validation (already performed)

On FreeBSD 15.1 with a live `wlan0` managed by the base wpa_supplicant, the
following were verified without changing the association:

- daemon attached to the real control socket (datagram, not stream);
- initial STATUS produced `Device.State = 100`;
- IP4Config exposed the real address (192.168.2.225/24) and the resolv.conf
  nameservers;
- a real scan returned 9-10 nearby APs, with raw SSID bytes, strength and
  privacy flags;
- after the scan, `ActiveAccessPoint` resolved to the AP matching the current
  BSSID and its SSID.

The procedure is:

```sh
# private bus (see the integration test for a config example)
DBUS_SYSTEM_BUS_ADDRESS=unix:path=/tmp/nmtest/bus \
  ./target/debug/nm-freebsd-daemon \
    --backend wpa-ctrl \
    --wpa-ctrl-dir /var/run/wpa_supplicant \
    --state-dir /tmp/nm-state \
    --wifi-interface wlan0 \
    --no-initial-scan --log-level debug
```

Then use `gdbus`:

```sh
gdbus call --system --dest org.freedesktop.NetworkManager \
  --object-path /org/freedesktop/NetworkManager \
  --method org.freedesktop.NetworkManager.GetDevices
```

Read `Device.State`, `Device.Wireless.ActiveAccessPoint`, AP properties and
`Device.Ip4Config` through `org.freedesktop.DBus.Properties.GetAll`. The
`live_client.py` used during development can be reconstructed from those calls;
the repository's integration test covers the same paths deterministically.

## Desktop rendering test

`scripts/desktop-test.sh` verifies the XFCE integration end to end without
touching the current `:0` session:

1. starts Xvfb on a free display;
2. starts a private session bus with `dbus-launch` and `xfconfd`;
3. starts a private system bus, a fake datagram wpa_supplicant and the real
   daemon;
4. installs the real plugin into a temporary HOME and writes a one-panel
   xfconf configuration;
5. starts the real `xfce4-panel`, waits for its window, captures the panel;
6. uses an XTest helper to click the plugin and captures the network menu.

Result on FreeBSD 15.1:

![panel](../artifacts/desktop-test/nm-freebsd-panel.png)

![menu](../artifacts/desktop-test/nm-freebsd-menu.png)

The button shows `Wi-Fi: Cafe WiFi`; the menu lists `Cafe WiFi 100% locked`,
`Lab 5G 74% locked`, `Open Net 46%`, Rescan and Disconnect. The fake
wpa_supplicant lives in `tools/fake-wpa.py`. Logs are copied to
`artifacts/desktop-test/logs/`.

This test requires `Xvfb`, `xfce4-panel`, `xfce4-screenshooter`, the XTest
headers (`pkg install libXtst`) and a real `wlan0` interface for the daemon's
device enumeration. It does not modify the real Wi-Fi association.

## Manual frontend checks

### Introspection

```sh
gdbus introspect --system --dest org.freedesktop.NetworkManager \
  --object-path /org/freedesktop/NetworkManager
gdbus introspect --system --dest org.freedesktop.NetworkManager \
  --object-path /org/freedesktop/NetworkManager/Settings
```

Confirm `StateChanged(u)`, `DeviceAdded(o)`, `GetDevices() -> ao`,
`AddAndActivateConnection(a{sa{sv}},o,o) -> oo`, and the `State` property.

### Settings CRUD

Use Python dbus or a small zbus client. The integration test is the reference
for the nested `a{sa{sv}}` construction. Check:

- `AddConnection` returns an object path and writes a 0600 JSON file;
- `GetConnection` returns `connection`, `802-11-wireless`, `ipv4`,
  `ipv6`;
- `GetSecrets` returns the psk only in the security subgroup;
- `Update` emits `Updated`;
- `Delete` emits `Removed` and removes the file.

### XFCE plugin

```sh
cargo build --release -p nm-xfce-plugin
nm -D target/release/libnm_freebsd_xfce.so | grep xfce_panel_module_init
readelf -d target/release/libnm_freebsd_xfce.so | grep -E 'xfce4panel|gtk-3'
cc -o /tmp/dlopen -x c - <<'EOF'
#include <dlfcn.h>
#include <stdio.h>
int main(void) {
    void *h = dlopen("target/release/libnm_freebsd_xfce.so", RTLD_NOW | RTLD_LOCAL);
    if (!h) { fprintf(stderr, "%s\n", dlerror()); return 1; }
    printf("%p\n", dlsym(h, "xfce_panel_module_init"));
    return 0;
}
EOF
/tmp/dlopen
```

The last check verifies that all dynamic dependencies resolve and the panel
entry point is exported. It does not start GTK or the panel.

Install the `.so` and generated `.desktop` as described in
[packaging/freebsd/README.md](../packaging/freebsd/README.md), then restart
`xfce4-panel` and add "NetworkManager FreeBSD" from *Add New Items*. The menu
should list real APs, and connecting should drive the same D-Bus path as the
integration test. If the panel is not available, compile and symbol checks are
the automated fallback.

### Live connect test (destructive; use a spare interface)

Only on a test machine or a spare wlan clone:

1. Save the current network configuration.
2. Start the daemon with `--backend wpa-ctrl`.
3. Call `AddAndActivateConnection` with a known SSID/PSK.
4. Confirm `wpa_cli -i <if> status` reaches `COMPLETED`.
5. Confirm `Device.State = 100`, active state 2, manager state 60.
6. Confirm `IP4Config.AddressData`, `Gateway` and DNS are populated. The
   daemon starts/reuses `dhclient` unless `--dhcp-backend` selects otherwise;
   use a spare interface and have a recovery path.
7. `DeactivateConnection`; confirm `DISCONNECTED` and that wpa_supplicant no
   longer reports the network as current.

Do not run this against a production uplink without a recovery path.

## One-click install verification

The install script was validated in two modes:

```sh
# staged install as an unprivileged user, no service changes
scripts/install.sh --destdir /tmp/stage --no-build --no-start --no-smoke
scripts/install.sh --destdir /tmp/stage --uninstall --yes

# show the planned system install without touching the system
scripts/install.sh --dry-run --no-build --no-start --no-smoke
```

The complete root install path was not executed in this environment because
root/sudo was unavailable; the service/start path is the same code that starts
the validated private daemon, and the staged tree was inspected for the exact
binary, policy, activation service, rc script and plugin paths.

## Troubleshooting

- **`Failed to own org.freedesktop.NetworkManager`**: another daemon owns the
  name, or the D-Bus policy was not reloaded. `service dbus restart` after
  installing the policy, and check `/var/log/messages`.
- **`Protocol wrong type for socket`**: something is using a stream socket
  against wpa_supplicant. The backend uses datagrams; if you are testing a
  custom build, verify with `truss`.
- **Permission denied on `/var/run/wpa_supplicant/wlan0`**: the daemon must run
  as root or as a user in `wheel` with group access. The port's rc script runs
  it as root.
- **`FAIL-BUSY` on scan**: wpa_supplicant is already scanning. `RequestScan`
  treats this as success.
- **Connected but no IP / CONNECTED_SITE**: check the selected DHCP backend,
  `/var/run/dhclient/dhclient.<if>.pid`, `/etc/dhclient-exit-hooks` and the
  lease timeout; see [FREEBSD-NOTES.md](FREEBSD-NOTES.md).
- **No APs**: check `wpa_cli -i wlan0 scan_results`; the daemon uses the same
  data. A new scan may take several seconds.
- **Plugin says "Wi-Fi unavailable"**: the daemon is not running, the bus policy
  blocks the user, or the name is owned by something else. Hover for the error
  text; the plugin retries every five seconds.

## CI notes

The integration test needs a `dbus-daemon` binary and a writable `/tmp`.
Unit tests include a live netlink dump on FreeBSD; they are gated with
`#[cfg(target_os = "freebsd")]` where appropriate. The XFCE plugin requires
GTK 3 and libxfce4panel development files, so a headless CI job should build
only `-p nm-freebsd-daemon` or provide the XFCE SDK.
