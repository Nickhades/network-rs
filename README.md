# nm-freebsd

**Languages:** [English](README.md) | [简体中文](README.zh-CN.md)

Repository: <https://github.com/Nickhades/network-rs>

A native FreeBSD implementation of the NetworkManager D-Bus service, written in
Rust. The daemon owns `org.freedesktop.NetworkManager` on the system bus and
serves the object tree that desktop frontends already understand. Real network
operations stay in FreeBSD: wpa_supplicant for Wi-Fi, netlink for links and
events, getifaddrs/resolv.conf for the addresses and DNS that are currently
configured.

The design rule is simple: **make the existing frontend work; do not write a
new desktop UI except where the desktop has no NetworkManager frontend of its
own.** plasma-nm and the GNOME network panel connect to the service directly.
XFCE gets a small panel plugin that is also only a D-Bus client.

```
plasma-nm / GNOME network panel / XFCE panel plugin
                     |
          system bus: org.freedesktop.NetworkManager
                     |
   +-----------------+------------------+
   |  zbus object layer                 |
   |  Manager, Device(.Wireless/.Wired),|
   |  AccessPoint, Settings,            |
   |  Settings.Connection, Active,     |
   |  IP4Config                        |
   +-----------------+------------------+
                     |
        core state + activation state machine
            /                       \
   wpa_supplicant                 FreeBSD netlink
   D-Bus or datagram ctrl         link dump/events
   socket                         getifaddrs + resolv.conf
```

## Status

Working, tested MVP on FreeBSD 15.1-RELEASE (amd64):

- end-to-end integration test against a private D-Bus daemon and a fake
  wpa_supplicant control socket;
- mock `fi.w1.wpa_supplicant1` integration test proving
  `RequestScan -> Interface.Scan -> ScanDone -> BSSs -> AccessPoint`;
- read-only validation against a live `wlan0` and the base-system
  wpa_supplicant (real scan returned nearby APs, real IPv4 address and DNS were
  published through IP4Config);
- XFCE panel plugin builds as a `libxfce4panel-2.0` module, loads in a real
  `xfce4-panel` under an isolated Xvfb display, and renders the status button
  and network menu (screenshots below);
- one-click install, dependency check, compile/test, read-only smoke test and
  isolated desktop-test scripts are included.

It has **not** yet been run against a live plasma-nm or GNOME network panel on
FreeBSD. Those frontends are the next compatibility milestone; see
[Limitations](#limitations-and-roadmap).

## What is implemented

| Area | Implemented |
| --- | --- |
| Manager | device list, state/connectivity, enable/sleep, permissions, state and device signals |
| Device | type, state/reason, MTU, MAC, active connection, IP4Config, managed/autoconnect |
| Device.Wireless | access points, request scan, active AP, last scan, capabilities |
| AccessPoint | SSID (raw bytes), strength, frequency, flags, WPA/RSN flags |
| Settings | list/get/add/update/delete/reload connections, NewConnection/ConnectionRemoved |
| Settings.Connection | get settings/secrets, update, update-unsaved, delete, Updated/Removed |
| Connection.Active | state, default, devices, IP4Config, IDs |
| IP4Config | address data, nameservers, DNS data (read-only) |
| Wi-Fi backend | open, WPA2-PSK, WPA3-SAE, scanning, status, connect, disconnect |
| Transports | wpa_supplicant datagram control socket (default on FreeBSD), optional `fi.w1.wpa_supplicant1` D-Bus backend |
| Link events | netlink RTM_GETLINK dump + multicast monitor, getifaddrs polling fallback |
| Desktop | KDE/GNOME/XFCE/unknown detection and logging; XFCE panel plugin |
| Packaging | D-Bus policy, activation service, rc.d script, port skeleton |

See [docs/DBUS.md](docs/DBUS.md) for signatures and frontend notes,
[docs/SCAN-FLOW.md](docs/SCAN-FLOW.md) for the exact RequestScan ->
Interface.Scan -> ScanDone -> BSSs chain, and
[docs/CODE-REVIEW.md](docs/CODE-REVIEW.md) for the review findings and accepted
risks. A bilingual documentation index is at [docs/README.md](docs/README.md).

## Important FreeBSD findings

These differ from the original architecture sketch and are handled in the code:

1. **Stock FreeBSD wpa_supplicant has no D-Bus interface.** The base-system
   binary is built without `CONFIG_CTRL_IFACE_DBUS`. The daemon therefore
   defaults to the native control interface, which on FreeBSD is a
   **`SOCK_DGRAM` Unix socket** requiring the client to bind its own path
   (verified with `truss(1)`). A D-Bus backend is still provided for
   ports/custom builds where `fi.w1.wpa_supplicant1` exists; `--backend auto`
   probes and falls back.
2. **The XFCE binding crate is named `xfce-panel`.** The upstream project is
   [xfce-panel-plugin-rs](https://github.com/d3v2a/xfce-panel-plugin-rs), but
   the published crates are `xfce-panel` and `xfce-panel-sys`. The plugin uses
   those.
3. **Desktop detection is local and tested.** The Linux-oriented
   `detect-desktop-environment` crate would add a dependency for reading two
   XDG variables. The same Kde/Gnome/Xfce/Unknown classification is implemented
   in `crates/nm-common/src/desktop.rs`, with unit tests and the same
   `XDG_CURRENT_DESKTOP` semantics.
4. **netlink is usable on FreeBSD 14+.** `crates/nm-freebsd-daemon/src/netlink.rs`
   speaks the FreeBSD wire layout directly, including FreeBSD's `sockaddr_nl`
   (which starts with a length byte, unlike Linux). If netlink is unavailable,
   interface list polling through getifaddrs keeps the daemon functional.

## One-click install and test

```sh
sudo scripts/install.sh                  # deps check, build, install, enable/start, smoke
sudo scripts/install.sh --install-deps   # optionally install missing pkg deps first
sudo scripts/install.sh --with-plugin --install-deps  # force plugin + its deps
make test                                # fmt + clippy -D warnings + tests + release build
make desktop                             # isolated Xvfb/XFCE screenshot test
```

Useful variants:

```sh
scripts/check-deps.sh --with-plugin      # read-only dependency report
scripts/build-test.sh                    # writes build-test.log
scripts/smoke-test.sh --scan             # read-only check of a running service
sudo scripts/install.sh --no-start       # install files but do not start
sudo scripts/install.sh --destdir /tmp/stage --no-start --no-smoke
scripts/install.sh --uninstall --yes     # stop and remove installed files
```

The installer uses the files in `packaging/freebsd`, backs up existing policy,
service and rc files as `*.nm-freebsd.bak`, enables the rc service, applies
the policy with `org.freedesktop.DBus.ReloadConfig` and runs the smoke test.
If Rust comes from rustup under your user, the installer derives
`CARGO_HOME`/`RUSTUP_HOME` for the root build; for development, build as
yourself and pass `--no-build`. It never removes `/var/db/nm-freebsd` unless
`--uninstall --purge` is used.

## Desktop rendering test

`scripts/desktop-test.sh` creates a disposable environment: Xvfb on a free
display, a private session bus, a private system bus, a fake wpa_supplicant,
the real daemon and the real XFCE panel. It installs the plugin into a temporary
HOME, writes a one-panel xfconf configuration, clicks the plugin with XTest and
captures both states. Your :0 session and panel configuration are not touched.

![XFCE panel plugin](artifacts/desktop-test/nm-freebsd-panel.png)

![Network menu](artifacts/desktop-test/nm-freebsd-menu.png)

The images show the real plugin loaded by `xfce4-panel` 4.20.8: the button
reads "Wi-Fi: Cafe WiFi"; the menu lists the fake APs with strength and lock
state plus Rescan/Disconnect. Logs are in
`artifacts/desktop-test/logs/`. The test needs `Xvfb`, `xfce4-panel`,
`xfce4-screenshooter`, XTest headers (`pkg install libXtst`) and a real
`wlan0` for device enumeration.

## Build

Requirements:

- FreeBSD 15.x (14.x should work; netlink is required for live events but the
  daemon falls back to polling);
- Rust 1.95 or newer (the XFCE binding requires edition 2024 / 1.95);
- `pkgconf`, GTK 3 and libxfce4panel 2.0 development files for the plugin;
- D-Bus (system bus) and wpa_supplicant for runtime.

```sh
cargo build --release -p nm-freebsd-daemon
cargo build --release -p nm-xfce-plugin   # optional, requires XFCE dev libs
cargo test --workspace
```

The daemon binary is `target/release/nm-freebsd-daemon`. The plugin is
`target/release/libnm_freebsd_xfce.so`; its generated desktop file lives under
`target/release/build/nm-xfce-plugin-*/out/nm_freebsd_xfce.desktop`.

## Run

The daemon must run as root: FreeBSD's wpa_supplicant control socket is
`root:wheel` mode 0770 and netlink link configuration needs privilege.

For a quick private-bus smoke test (does not touch the system bus policy):

```sh
# terminal 1: a private bus
dbus-daemon --config-file=/path/to/permissive-system.conf --nofork

# terminal 2
DBUS_SYSTEM_BUS_ADDRESS=unix:path=/path/to/bus \
  sudo -E target/release/nm-freebsd-daemon \
    --backend wpa-ctrl --wpa-ctrl-dir /var/run/wpa_supplicant
```

For production install the files from `packaging/freebsd`:

```sh
install -m 755 target/release/nm-freebsd-daemon /usr/local/bin/
install -d /usr/local/etc/dbus-1/system.d /usr/local/share/dbus-1/system-services
install -m 644 packaging/freebsd/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf \
  /usr/local/etc/dbus-1/system.d/
sed 's|%%PREFIX%%|/usr/local|g' \
  packaging/freebsd/etc/dbus-1/system-services/org.freedesktop.NetworkManager.service.in \
  > /usr/local/share/dbus-1/system-services/org.freedesktop.NetworkManager.service
sed 's|%%PREFIX%%|/usr/local|g' packaging/freebsd/etc/rc.d/nm_freebsd \
  > /usr/local/etc/rc.d/nm_freebsd
chmod 755 /usr/local/etc/rc.d/nm_freebsd
service dbus restart
sysrc nm_freebsd_enable=YES
service nm_freebsd start
```

The port skeleton is in
`packaging/freebsd/ports/net-mgmt/nm-freebsd`; see
[packaging/freebsd/README.md](packaging/freebsd/README.md). The port's
`cargo-crates.mk` is generated from `Cargo.lock` by
[tools/gen-port-crates.py](tools/gen-port-crates.py).

### Daemon options

```
--backend auto|wpa-ctrl|wpa-dbus   wpa_supplicant transport (default auto)
--wpa-ctrl-dir PATH                default /var/run/wpa_supplicant
--state-dir PATH                   profile storage, default /var/db/nm-freebsd
--wifi-interface NAME              repeatable; auto-detected wlan* otherwise
--no-wired                         do not expose Ethernet interfaces
--bus system|session               default system
--bus-address ADDRESS              explicit D-Bus address (jails/tests)
--desktop auto|kde|gnome|xfce|unknown
--no-initial-scan                  skip the startup Wi-Fi scan
--log-level FILTER                 env NM_FREEBSD_LOG also works
```

## Desktop behaviour

- **KDE / GNOME:** the daemon starts no UI. plasma-nm or the GNOME panel owns
  the user experience and talks to the D-Bus service.
- **XFCE:** after the port is installed with the XFCE option enabled,
  "NetworkManager FreeBSD" appears in *Panel → Add New Items*. The plugin only
  needs D-Bus; it shows the
  current state, scans on demand, lists networks, asks for a WPA passphrase and
  calls `AddAndActivateConnection`.
- **Unknown:** D-Bus only; the daemon logs a hint. Set `XDG_CURRENT_DESKTOP` or
  `--desktop` for headless images.

The XFCE plugin is session-scoped and the daemon is system-scoped. Every plugin
request opens a fresh system-bus connection, so restarting the daemon does not
require restarting the panel.

## Profile storage

Profiles are small JSON documents in `/var/db/nm-freebsd/connections/*.json`
(directory 0700, files 0600). Only fields this daemon can apply are persisted:
UUID, id, type, SSID (byte array), passphrase, key management, autoconnect and
timestamp. The D-Bus settings dictionaries are reconstructed from those fields.
Secrets are not logged.

## Security

There is no polkit integration in this MVP. The installed D-Bus policy allows
all local users to call the service; only root may own the bus name. The daemon
runs as root, so a local user can change the machine's Wi-Fi configuration.
For multi-user or hostile-local-user systems, tighten
`/usr/local/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf` (the file
contains a commented group-based alternative). `GetPermissions` reports
"yes" for the implemented operations to match the policy.

Do not expose the system bus to untrusted machines. Profiles contain Wi-Fi
passphrases; treat `/var/db/nm-freebsd` as secret.

## Limitations and roadmap

Deliberately out of scope for this MVP:

- **DHCP/address configuration and routes are not applied by the daemon.**
  After association the daemon publishes the address that FreeBSD already has
  (for example from `ifconfig_wlan0="WPA DHCP"`). A system without a DHCP
  client will associate but have no IPv4 address. Gateway is left empty.
- Wired activation is not implemented; Ethernet devices are visible with link
  state and any existing IPv4/IP4Config, but `ActivateConnection` on them
  returns `NotSupported`.
- No IPv6 (`IP6Config` is `/`), no DHCP option objects, no secret agent,
  no polkit, no VPN, no enterprise (802.1X), no WEP, no hidden-network UI.
- No automatic activation of saved profiles. wpa_supplicant's own configured
  networks may still associate; the daemon will report that state.
- A Wi-Fi link that was already up when the daemon started is reported as
  `CONNECTED_SITE` with no `ActiveConnection` object until an activation is
  performed through the daemon.
- Access points are only removed when a non-empty scan no longer contains them;
  an empty scan result preserves the previous list to avoid flicker.
- The XFCE plugin polls every 5 seconds and does not subscribe to signals.
- Live plasma-nm and GNOME panel testing is still pending.

## Repository map

```
crates/nm-common/          constants, desktop detection, SSID/security helpers
crates/nm-freebsd-daemon/  daemon, D-Bus objects, wpa_ctrl/wpa_dbus, netlink
  tests/integration.rs     private bus + fake wpa_supplicant end-to-end test
crates/nm-xfce-plugin/     XFCE panel plugin (cdylib)
scripts/                   install, deps, build-test, smoke-test, desktop-test
packaging/freebsd/         policy, activation service, rc.d, port skeleton
tools/                     fake wpa server, port crate-list generator
artifacts/desktop-test/    panel/menu screenshots and logs
docs/                      English architecture, D-Bus, scan flow, FreeBSD, testing, review
docs/zh-CN/                Chinese mirrors of the same documents
Makefile                   convenience targets for the scripts
```

## License

BSD-2-Clause. See [LICENSE](LICENSE).
