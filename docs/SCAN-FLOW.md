# Wi-Fi scan flow

**Languages:** [English](SCAN-FLOW.md) | [简体中文](zh-CN/SCAN-FLOW.md)

The upper and lower interfaces are intentionally different layers:

```
desktop frontend
  |  org.freedesktop.NetworkManager.Device.Wireless.RequestScan(a{sv})
  v
nm-freebsd Core::request_scan
  |
  v
Backend::scan
  |                                   \
  v                                    v
wpa_supplicant D-Bus               wpa_supplicant ctrl socket
fi.w1.wpa_supplicant1.Interface    (stock FreeBSD)
  Interface.Scan(a{sv})              SCAN
  Interface.ScanDone(b)              CTRL-EVENT-SCAN-RESULTS
  Interface.BSSs (a{oa{sv}})         SCAN_RESULTS
  BSSAdded / BSSRemoved
  |                                    |
  +----------------+-------------------+
                   v
       ScanAp -> AccessPoint D-Bus objects
       AccessPointAdded / AccessPointRemoved
       PropertiesChanged
```

## Upper layer: NetworkManager

The frontend calls:

```text
org.freedesktop.NetworkManager.Device.Wireless.RequestScan(a{sv} options)
```

The method returns immediately. Results arrive later through:

- the `AccessPoints` property;
- `AccessPointAdded(o)` / `AccessPointRemoved(o)`;
- `org.freedesktop.DBus.Properties.PropertiesChanged`.

The daemon maps each BSS to a stable AccessPoint object at
`/org/freedesktop/NetworkManager/AccessPoints/<counter>`. SSIDs are exposed as
raw `ay` bytes, with `Strength` 0-100, frequency, privacy and WPA/RSN flags.

## Lower layer: wpa_supplicant D-Bus

The real D-Bus interface is:

```text
fi.w1.wpa_supplicant1
  /fi/w1/wpa_supplicant1/Interfaces/<n>
    Interface.Scan(a{sv} args)
    signal Interface.ScanDone(b success)
    property Interface.BSSs a{oa{sv}}
    signal Interface.BSSAdded(o bss, a{sv} properties)
    signal Interface.BSSRemoved(o bss)
```

There is **no `GetScanResults` method** in the wpa_supplicant D-Bus API. The
control-socket command is called `SCAN_RESULTS`; on D-Bus the equivalent data is
the `BSSs` property. `BSSAdded`/`BSSRemoved` are also available for
incremental updates.

The implementation in `backend/wpa_dbus.rs`:

1. subscribes to `ScanDone`;
2. `Backend::scan` calls `Interface.Scan`;
3. on `ScanDone(true)`, sends `BackendEvent::ScanResults`;
4. `Backend::scan_results` reads `Interface.BSSs` and converts each dictionary
   entry to `ScanAp`;
5. the core diffs the new list against existing AP objects and emits
   `AccessPointAdded`/`AccessPointRemoved` plus `PropertiesChanged`.

A slow ten-second BSS poll is kept only as a fallback if a ScanDone signal was
missed.

## Stock FreeBSD: control socket

FreeBSD's base wpa_supplicant is built without
`CONFIG_CTRL_IFACE_DBUS`, so `fi.w1.wpa_supplicant1` does not exist. The default
backend therefore uses the datagram control socket:

1. `Backend::scan` sends `SCAN`;
2. the event socket receives `CTRL-EVENT-SCAN-RESULTS`;
3. the backend sends `SCAN_RESULTS` and parses the tab-separated BSS list;
4. the same core diff/AP-object path runs.

The two transports converge on the same `BackendEvent::ScanResults` and
`Backend::scan_results` abstraction, so the NetworkManager layer does not know
which one is in use.

## Verification

`crates/nm-freebsd-daemon/tests/wpa_dbus_mock.rs` starts a mock
`fi.w1.wpa_supplicant1` service on a private system bus and proves:

1. a direct `Interface.Scan` causes `ScanDone` and the daemon populates
   NetworkManager AccessPoint objects from `BSSs`;
2. the upper `RequestScan` reaches `Interface.Scan` (the mock's scan counter
   increments).

The normal end-to-end test covers the control-socket path, and the live
read-only test runs against the real FreeBSD `wlan0`.

## Current limitation

`RequestScan` accepts the options dictionary but directed scans
(`options["ssids"]`) are not forwarded yet. A request with an empty dictionary
is fully supported; per-SSID directed scans are a roadmap item.
