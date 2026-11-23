# FreeBSD implementation notes

## wpa_supplicant has no D-Bus on stock FreeBSD

The architecture sketch assumed `fi.w1.wpa_supplicant1`. The base-system
wpa_supplicant v2.11 on FreeBSD 15.1 is built without
`CONFIG_CTRL_IFACE_DBUS`: `strings` finds no `fi.w1` service name. The daemon
therefore defaults to the control socket and keeps a D-Bus backend for ports
builds.

The D-Bus backend (used only when `fi.w1.wpa_supplicant1` exists) follows the
real API: `Interface.Scan` -> `ScanDone` -> `BSSs`, not a
`GetScanResults` method. A mock service test covers this path; see
[SCAN-FLOW.md](SCAN-FLOW.md).

The control interface is also not a stream socket. Tracing `wpa_cli` with
`truss(1)` shows:

```
socket(PF_LOCAL,SOCK_DGRAM,0) = 3
bind(3,{ AF_UNIX "/tmp/wpa_ctrl_15764-1" },106) = 0
connect(3,{ AF_UNIX "/var/run/wpa_supplicant/wlan0" },106) = 0
sendto(3,"STATUS",6,0,NULL,0) = 6
recvfrom(3,"bssid=04:8b:..."...,4095,0,NULL,0x0) = 249
```

Consequences implemented in `backend/wpa_ctrl.rs`:

- client socket is `SOCK_DGRAM` and binds a unique private path under
  `std::env::temp_dir()` (mode 0600);
- commands are sent without a trailing newline;
- every reply is a datagram, not a stream, so no line-framing heuristic is
  needed;
- `ATTACH` keeps the client socket alive and events arrive as later datagrams;
- the client path is removed on drop.

The first version used `tokio::net::UnixStream` and passed against a stream
fake. Real hardware immediately returned `EPROTOTYPE` and exposed the bug. The
fake server in the integration test now uses `UnixDatagram` too.

## netlink

FreeBSD 14+ provides netlink(4). The daemon uses it for:

- an initial `RTM_GETLINK` dump;
- multicast `RTMGRP_LINK` / `RTMGRP_IPV4_IFADDR` notifications.

FreeBSD's wire layout is Linux-like but not identical. In particular
`struct sockaddr_nl` starts with `nl_len` and `AF_NETLINK` is 38. Constants were
taken from `/usr/include/netlink`. The implementation parses TLVs directly and
does not depend on Linux-only crates.

If the monitor socket cannot be created, the daemon falls back to a five-second
getifaddrs poll. Interface state, MAC, MTU and IPv4 addresses are still
available. IPv4 addresses and DNS servers are read with getifaddrs and
`/etc/resolv.conf`; the daemon does not run a DHCP client.

## D-Bus policy and activation

Files:

- `packaging/freebsd/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf`
- `packaging/freebsd/etc/dbus-1/system-services/org.freedesktop.NetworkManager.service.in`
- `packaging/freebsd/etc/rc.d/nm_freebsd`

Installed paths on FreeBSD:

```
/usr/local/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf
/usr/local/share/dbus-1/system-services/org.freedesktop.NetworkManager.service
/usr/local/etc/rc.d/nm_freebsd
```

The policy allows root to own the name and all local users to send to / receive
from it. The service file lets the bus start the daemon on demand as root. The
rc.d script uses `/usr/sbin/daemon -p` to background the daemon and writes
`/var/run/nm_freebsd.pid`.

The FreeBSD dbus rc script has no reload command, so `scripts/install.sh`
calls `org.freedesktop.DBus.ReloadConfig` through `dbus-send` after installing
the policy. This re-reads policy and service directories without disconnecting
desktop clients. If a locally modified bus configuration ignores the reload,
the installer prints the `service dbus restart` fallback.

The daemon needs root for the wpa_supplicant socket and for interface changes.
There is no privilege-separated helper in this MVP.

## XFCE panel plugin ABI

The upstream project [xfce-panel-plugin-rs](https://github.com/d3v2a/xfce-panel-plugin-rs)
publishes the crates `xfce-panel` and `xfce-panel-sys`. The plugin:

- is a `cdylib` named `libnm_freebsd_xfce.so`;
- exports `xfce_panel_module_init` (verified with `nm -D`);
- links `libxfce4panel-2.0.so.4`, GTK 3 and GLib;
- generates its `.desktop` via the crate's `build` feature in `build.rs`.

Install paths:

```
/usr/local/lib/xfce4/panel/plugins/libnm_freebsd_xfce.so
/usr/local/share/xfce4/panel/plugins/nm_freebsd_xfce.desktop
```

The panel scans these directories. The plugin runs inside the panel process, so
do not overwrite the `.so` in place while it is loaded; install to a temporary
name and rename, then restart `xfce4-panel`.

The panel plugin's D-Bus worker is independent of the daemon lifetime. Every
request opens a fresh connection, so a daemon restart is recovered on the next
five-second refresh or user action.

## Port

`packaging/freebsd/ports/net-mgmt/nm-freebsd` is a port skeleton. Before
building it in a real ports tree:

- set `GH_ACCOUNT`/`MAINTAINER` to the real upstream;
- run `make makesum` to create `distinfo` (the skeleton intentionally does not
  contain a fabricated source checksum);
- regenerate `cargo-crates.mk` with
  `python3 tools/gen-port-crates.py > packaging/freebsd/ports/net-mgmt/nm-freebsd/cargo-crates.mk`.

The XFCE option pulls GTK 3 and `sysutils/xfce4-panel`; disable it for headless
systems. `wpa_supplicant` is part of the base system, so there is no port
dependency for it.
