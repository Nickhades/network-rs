# Architecture

**Languages:** [English](ARCHITECTURE.md) | [简体中文](zh-CN/ARCHITECTURE.md)

## Layers

1. **Desktop dispatch** (`crates/nm-common/src/desktop.rs`,
   `crates/nm-freebsd-daemon/src/daemon.rs`) reads the XDG desktop variables
   once and logs which frontend is expected. It never starts a UI. KDE and
   GNOME use their native frontends; XFCE uses the plugin installed by the
   port; unknown desktops get D-Bus only.
2. **D-Bus service** (`crates/nm-freebsd-daemon/src/dbus/`) is the facade. zbus
   owns `org.freedesktop.NetworkManager`, exports the manager, device,
   access-point, settings, active-connection and IP4Config objects, and emits
   NetworkManager-shaped signals.
3. **Core** (`crates/nm-freebsd-daemon/src/core.rs`) holds the canonical state,
   performs object lifecycle (register/unregister dynamic APs and
   connections), runs the activation state machine and emits notifications.
4. **Backends** (`crates/nm-freebsd-daemon/src/backend/`) translate commands
   into wpa_supplicant operations. `netlink.rs` and `system.rs` provide FreeBSD
   link/event/address facts. No command-line parsing of `ifconfig` is used.

The desktop plugin is a separate cdylib and a pure D-Bus client. It shares only
the constant/desktop/SSID helpers from `nm-common`.

## State model

`CoreState` (guarded by `tokio::sync::RwLock`) contains:

- `devices: HashMap<ifname, DeviceEntry>`
- `aps: HashMap<"ifname/bssid", ApEntry>`
- `connections: HashMap<OwnedObjectPath, ConnectionProfile>`
- `active: HashMap<OwnedObjectPath, ActiveEntry>`
- manager flags (state, connectivity, networking/wireless enabled, primary)
- monotonic counters for AP, connection, active-connection and IP4Config
  object paths.

Object paths are stable for the lifetime of an object:

```
/org/freedesktop/NetworkManager
/org/freedesktop/NetworkManager/Devices/<ifindex>
/org/freedesktop/NetworkManager/AccessPoints/<counter>
/org/freedesktop/NetworkManager/Settings
/org/freedesktop/NetworkManager/Settings/<counter>
/org/freedesktop/NetworkManager/ActiveConnection/<counter>
/org/freedesktop/NetworkManager/IP4Config/<counter>
```

The device path uses the interface index, so a device re-added with the same
index reuses its path. APs are keyed by interface + BSSID; the same SSID from
different BSSIDs remains distinct, as in NetworkManager.

## Deadlock avoidance

The first implementation emitted property changes with
`ObjectServer::interface(..).get_mut()`. That deadlocks when called from inside
a D-Bus method: the dispatcher already holds a read lock on the interface and
the handler then waits for a write lock on the same object.

All notifications are therefore emitted directly through
`SignalEmitter::new(&connection, path)`:

- `org.freedesktop.DBus.Properties.PropertiesChanged` is constructed manually
  from a state snapshot;
- manager `StateChanged`, device `StateChanged`, access point added/removed,
  settings new/removed and active-connection notifications use
  `SignalEmitter::emit`.

This also keeps the object server out of hot paths.

The `State` property uses
`#[zbus(property(emits_changed_signal = "false"))]` so the macro would not
generate a `state_changed` helper that collides with the `StateChanged`
signal. The same manual PropertiesChanged dictionary covers it.

## Backend abstraction

`backend::Backend` is an async trait:

```rust
async fn scan(ifname) -> Result<(), BackendError>;
async fn scan_results(ifname) -> Result<Vec<ScanAp>, BackendError>;
async fn status(ifname) -> Result<WifiStatus, BackendError>;
async fn connect(&ConnectRequest) -> Result<(), BackendError>;
async fn disconnect(ifname) -> Result<(), BackendError>;
fn spawn_events(Arc<Self>, Sender<BackendEvent>);
```

Two implementations:

- `wpa_ctrl`: datagram Unix socket, the only transport available in stock
  FreeBSD. Commands use a short-lived bound client socket; events use one
  long-lived socket after `ATTACH`. See
  [FREEBSD-NOTES.md](FREEBSD-NOTES.md).
- `wpa_dbus`: `fi.w1.wpa_supplicant1` proxies, for custom builds. It calls
  `Interface.Scan`, listens for `ScanDone`, then reads the `BSSs` property;
  state is polled every two seconds and a ten-second BSS poll is a fallback.
  Auto mode only selects it if the bus name is owned.

`Auto` probes for the D-Bus name and otherwise chooses the control socket.
An explicit `--backend` is never silently downgraded except in auto mode.

## Activation

`AddAndActivateConnection` and `ActivateConnection` follow the same core path:

1. Resolve or create a profile. A `/` connection path with a specific AP builds
   an unsaved profile from the AP's SSID and security.
2. Resolve the device. `/` means the first Wi-Fi device matching the profile.
3. Deactivate an existing active connection on that device.
4. Insert an `ActiveEntry` in `ACTIVATING` and set the device to `PREPARE`.
5. Register `Connection.Active`, then spawn the activation task.
6. The task sets `CONFIG`, calls `Backend::connect`, and polls
   `Backend::status` every 500 ms for up to 20 s.
7. On `COMPLETED`: set `ACTIVATED`, publish IP4Config if an address exists,
   mark the active connection `ACTIVATED` and default, recompute the manager
   state, emit device/active/manager signals.
8. On failure/timeout/disconnect: mark the device `FAILED` with a NetworkManager
   state reason, emit, then remove the active object and return to
   `DISCONNECTED`.

`DeactivateConnection` sets `DEACTIVATING`, calls `Backend::disconnect`, removes
the IP4Config and active objects, clears device state and recomputes the
manager. The activation task checks that its active path still exists before
every update, so a concurrent deactivate cancels it naturally.

## Events

Two pumps feed the core:

- **wpa_supplicant events:** `CTRL-EVENT-SCAN-RESULTS` triggers AP refresh;
  connected/disconnected/auth events trigger a STATUS query and a state update.
- **netlink:** link add/delete/change updates device presence and state; address
  changes refresh IPv4 information. If the netlink monitor cannot be created,
  a five-second getifaddrs poll is used.

AP refresh is serialized by a mutex. New APs are registered as objects before
`AccessPointAdded` is emitted; removed APs are unregistered after
`AccessPointRemoved`. An empty scan keeps the old list to avoid UI flicker.

## Profiles

Connection profiles are JSON files, not serialized zvariant maps. Only the
fields the backend can apply are stored; `GetSettings` synthesizes the standard
`a{sa{sv}}` dictionary, and `GetSecrets` returns the security subgroup. This
keeps the on-disk format inspectable and avoids persisting arbitrary untrusted
variant data.

## Desktop plugin lifecycle

The XFCE plugin is loaded by xfce4-panel inside the panel process. It creates a
GTK button and a worker thread with a current-thread Tokio runtime. The worker
owns all zbus work; the GTK main thread only consumes `UiEvent` values via a
150 ms timeout source.

Each request opens a fresh system-bus connection. If the daemon restarts, the
next five-second refresh or menu action reconnects; the plugin has no sticky
connection to break. On plugin removal, timer sources and the worker shutdown
request are released.
