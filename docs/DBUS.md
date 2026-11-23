# D-Bus surface

**Languages:** [English](DBUS.md) | [简体中文](zh-CN/DBUS.md)

Service: `org.freedesktop.NetworkManager` (system bus)

All object paths below are served by zbus. Types are the D-Bus signatures
reported by introspection.

## Manager

Path: `/org/freedesktop/NetworkManager`

Properties:

| Name | Type | Notes |
| --- | --- | --- |
| Version | s | `1.46.0` |
| State | u | NMState; 50 for an externally established link |
| Startup | b | false |
| NetworkingEnabled | b | |
| WirelessEnabled | b | read/write |
| WirelessHardwareEnabled | b | true when a Wi-Fi device exists |
| Connectivity | u | 4 full, 3 limited, 1 none, 0 unknown |
| Devices | ao | all exposed devices |
| ActiveConnections | ao | |
| PrimaryConnection | o | `/` when none |
| ActivatingConnection | o | `/` when none |
| Metered | u | 0 unknown |
| GlobalDnsConfiguration | a{sv} | empty |

Methods:

| Method | Signature |
| --- | --- |
| GetDevices | () -> ao |
| GetAllDevices | () -> ao |
| GetDeviceByIpIface | (s) -> o |
| ActivateConnection | (o connection, o device, o specific) -> o |
| AddAndActivateConnection | (a{sa{sv}}, o device, o specific) -> (o, o) |
| DeactivateConnection | (o) -> () |
| Enable | (b) -> () |
| Sleep | (b) -> () |
| CheckConnectivity | () -> u |
| GetPermissions | () -> a{ss} |
| SetLogging | (s, s) -> () |
| Reload | () -> () |

Signals: `StateChanged(u)`, `DeviceAdded(o)`, `DeviceRemoved(o)`, plus
`PropertiesChanged` on `org.freedesktop.DBus.Properties`.

## Device

Path: `/org/freedesktop/NetworkManager/Devices/<ifindex>`

Properties include `Udi`, `Interface`, `IpInterface`, `Driver`,
`DriverVersion`, `FirmwareVersion`, `Capabilities`, `State`, `StateReason`
(`(uu)`), `ActiveConnection`, `AvailableConnections`, `Autoconnect`,
`Managed`, `DeviceType`, `Dhcp4Config`, `Ip4Config`, `Ip6Config`, `Mtu`,
and the deprecated `Ip4Address`.

Methods: `Reapply(a{sa{sv}}, t, u)` and `Delete()` return
`NotSupported`; they are present so frontends can introspect and fail cleanly.

Signal: `StateChanged(u new_state, u old_state, u reason)`.

### Device.Wireless

Properties: `HwAddress`, `PermHwAddress`, `Mode`, `Bitrate`, `AccessPoints`
(ao), `ActiveAccessPoint`, `WirelessCapabilities`, `LastScan` (x).

The lower-layer scan mapping (including why wpa_supplicant D-Bus uses `BSSs`
rather than a `GetScanResults` method) is documented in
[SCAN-FLOW.md](SCAN-FLOW.md).

Methods:

- `GetAccessPoints() -> ao` (deprecated)
- `GetAllAccessPoints() -> ao`
- `RequestScan(a{sv} options)`

Signals: `AccessPointAdded(o)`, `AccessPointRemoved(o)`.

### Device.Wired

`HwAddress`, `PermHwAddress`, `Speed`, `Carrier`. Wired devices are
read-only in this MVP.

## AccessPoint

Path: `/org/freedesktop/NetworkManager/AccessPoints/<counter>`

`Flags` (u), `WpaFlags` (u), `RsnFlags` (u), `Ssid` (ay, raw bytes),
`Frequency` (u), `HwAddress` (s), `Mode` (u), `MaxBitrate` (u),
`Strength` (y, 0-100), `LastSeen` (i).

## Settings

Path: `/org/freedesktop/NetworkManager/Settings`

Properties: `Hostname` (s), `CanModify` (b).

Methods:

| Method | Signature |
| --- | --- |
| ListConnections | () -> ao |
| GetConnection | (o) -> a{sa{sv}} |
| AddConnection | (a{sa{sv}}) -> o |
| AddConnectionUnsaved | (a{sa{sv}}) -> o |
| UpdateConnection | (o, a{sa{sv}}) -> b |
| DeleteConnection | (o) -> () |
| ReloadConnections | () -> b |
| SaveHostname | (s) -> () |

Signals: `NewConnection(o)`, `ConnectionRemoved(o)`.

### Settings.Connection

Path: `/org/freedesktop/NetworkManager/Settings/<counter>`

- `GetSettings() -> a{sa{sv}}` (without secrets)
- `GetSecrets(s service) -> a{sa{sv}}` (only the security subgroup)
- `Update(a{sa{sv}})`
- `UpdateUnsaved(a{sa{sv}})`
- `Delete()`

Signals: `Updated()`, `Removed()`.

### Settings dictionary example

The daemon accepts and emits the standard nested dictionary. A WPA2 profile
looks like this in GVariant text form:

```
{
  'connection': {'id': <'Cafe WiFi'>, 'uuid': <'...'>,
                 'type': <'802-11-wireless'>, 'autoconnect': <true>},
  '802-11-wireless': {'ssid': <[byte 0x43, 0x61, 0x66, 0x65, ...]>,
                      'mode': <'infrastructure'>},
  '802-11-wireless-security': {'key-mgmt': <'wpa-psk'>,
                               'psk': <'hunter2hunter2'>},
  'ipv4': {'method': <'auto'>},
  'ipv6': {'method': <'auto'>}
}
```

Open networks use `key-mgmt = 'none'` or omit the security setting entirely.
SAE is accepted as `key-mgmt = 'sae'`.

## Connection.Active

Path: `/org/freedesktop/NetworkManager/ActiveConnection/<counter>`

Properties: `Connection` (o), `SpecificObject` (o), `Id` (s), `Uuid` (s),
`Type` (s), `Devices` (ao), `State` (u), `StateFlags` (u), `Default` (b),
`Default6` (b), `Vpn` (b), `Master` (o), `Ip4Config` (o),
`Dhcp4Config` (o), `Ip6Config` (o), `Dhcp6Config` (o).

## IP4Config

Path: `/org/freedesktop/NetworkManager/IP4Config/<counter>`

`AddressData` (aa{sv}) with `address`/`prefix`, `Addresses` (aau),
`Gateway` (s, currently empty), `NameserverData` (aa{sv}), `Nameservers`
(au), `Domains` (as), `Routes` (aa{sv}), `DnsOptions` (as),
`DnsPriority` (i).

The object is created when a Wi-Fi activation gets an address, when a wired
link has an existing IPv4 address, or when an externally established link is
observed with an address. It reads live device state, so a DHCP lease change is
visible without recreating the object.

## Frontend call paths

A typical plasma-nm/GNOME scan/connect sequence maps to:

1. `Manager.GetDevices` or the `Devices` property.
2. `org.freedesktop.DBus.Properties.GetAll` on each device to find
   `DeviceType == 2`.
3. `Device.Wireless.GetAllAccessPoints`; then `GetAll`/property reads on each
   `AccessPoint` (`Ssid`, `Strength`, `Flags`, `WpaFlags`, `RsnFlags`).
4. `Device.Wireless.RequestScan`.
5. `Manager.AddAndActivateConnection(settings, device, "/")`.
6. Watch `Connection.Active.State` and `Device.State`; read `IP4Config` when
   activated.
7. `Manager.DeactivateConnection(active)`.

`GetPermissions` resolves the caller's Unix uid from the bus daemon. Members
of `wheel` or `network` get `yes` for control operations; other local users get
`no` (scan remains `yes`). Unsupported features (WWAN, Wi-Fi sharing, per-user
settings) report `no` even for administrators, so a frontend does not show dead
controls. The D-Bus policy grants the same groups full access and gives
unprivileged users read-only calls.

There is no `org.freedesktop.NetworkManager.SecretAgent` or `AgentManager`
implementation; send the PSK in the settings dictionary (as the XFCE plugin
does). Frontends that insist on an agent for WPA-PSK will need that follow-up
work. Saved PSKs are read from the separate `keyring.json`; `GetSecrets`
requires an administrator caller.

## Manual inspection

```sh
gdbus introspect --system \
  --dest org.freedesktop.NetworkManager \
  --object-path /org/freedesktop/NetworkManager

gdbus call --system --dest org.freedesktop.NetworkManager \
  --object-path /org/freedesktop/NetworkManager \
  --method org.freedesktop.NetworkManager.GetDevices

gdbus call --system --dest org.freedesktop.NetworkManager \
  --object-path /org/freedesktop/NetworkManager/Devices/3 \
  --method org.freedesktop.NetworkManager.Device.Wireless.RequestScan '{}'
```

(`busctl` is a systemd tool and is not available on FreeBSD; `gdbus`,
`dbus-send` and Python dbus work.)
