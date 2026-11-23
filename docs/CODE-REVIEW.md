# Code review

**Languages:** [English](CODE-REVIEW.md) | [简体中文](zh-CN/CODE-REVIEW.md)

## 中文摘要

本轮审核覆盖 Rust 守护进程/插件、安装与测试脚本、FreeBSD 打包。重点修复了
wpa_supplicant 控制接口的 datagram 类型、D-Bus 方法内自死锁、非 ASCII SSID
的 wpa 转义、配置文件 UUID 路径穿越、插件读取 `/dev/urandom` 阻塞、激活/断开
竞态、首次激活 IP4Config 缺失和有线接口状态等问题。Phase 2 已补上
dhclient/dhcpcd DHCP、exit-hook 租约事件、Gateway/DNS、有线激活和
CONNECTED_SITE 超时语义。剩余主要缺口是 SecretAgent/polkit、IPv6、dhcpcd 专用
lease hook，以及尚未在真实 plasma-nm/GNOME 面板上验证。自动测试、私有总线冒烟、
真实 wpa 只读扫描和隔离 XFCE 面板渲染均已通过。

Scope: the Rust workspace, shell install/test tooling and FreeBSD packaging in
this repository. This is a working review, not a formal security audit.

## Verification at review time

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`: 16 common + 33 daemon + 1 plugin + 2 end-to-end
  integration tests (control socket and mock wpa D-Bus)
- release build of daemon and XFCE plugin
- plugin `dlopen` + `xfce_panel_module_init` symbol check
- staged one-click install/uninstall under `--destdir`
- read-only smoke test against a private system bus, fake wpa_supplicant and the
  real wpa_supplicant
- isolated Xvfb/XFCE panel rendering test with panel and menu screenshots
  (`artifacts/desktop-test/`)

## Findings fixed

| Severity | Area | Finding | Fix |
| --- | --- | --- | --- |
| Critical | wpa backend | FreeBSD wpa_supplicant control sockets are `SOCK_DGRAM`; the original stream client failed with `EPROTOTYPE` on real hardware. | Datagram client with private client bind, newline-free commands and per-datagram replies; test double updated. |
| High | D-Bus notifications | Emitting property changes with `ObjectServer::interface(..).get_mut()` from inside a method deadlocked against the dispatcher's read lock. | All notifications use `SignalEmitter` directly with explicit `PropertiesChanged` dictionaries. |
| High | XFCE plugin | `std::fs::read("/dev/urandom")` reads until EOF and would block the worker forever. | Open the file and `read_exact` 16 bytes, with a time fallback. |
| High | Profiles | The client-supplied UUID became a filename, allowing path traversal (`../../...`). | UUID is parsed/regenerated on input, validated on load and save; tests cover both. |
| High | Secrets | Wi-Fi PSKs were stored inside the profile JSON next to non-secret metadata. | Move secrets to a separate 0600 `keyring.json`, migrate legacy `psk` fields, and keep profile JSON secret-free. |
| High | Authorization | The policy allowed all local users full method calls and `GetPermissions` always said yes. | Full access limited to root/`wheel`/`network`; daemon checks caller uid on mutating methods; `GetPermissions` resolves the uid and reports read-only for others. |
| Medium | Profiles | Adding a second profile with an existing UUID created two paths and could overwrite the same JSON file. | Reuse the existing path and emit `Updated`. |
| Medium | Activation | Deactivate could race the activation task between its last check and the state write, leaving a device marked ACTIVATED. | The completion/failure paths re-check that the active object still exists and is not DEACTIVATED inside the write lock. |
| Medium | IP4Config | Activation created IP4Config before copying the new address into the device, so the object was never created on first connect. | Publish the address before `ensure_ip4_config`; remove stale objects on disconnect. |
| Medium | Wired links | Ethernet devices with an existing IPv4 address were reported as unavailable and had no IP4Config. | Link/address updates set ACTIVATED and create/remove IP4Config. |
| Medium | Manager state | Manager state was not recomputed when a wired device appeared/activated. | Recompute in upsert and on address changes. |
| Medium | XFCE plugin | Click closure formed an `Rc` cycle with the button; timers survived plugin removal. | Weak reference in the closure; `SourceId` timers removed in `free_data`. |
| Low | D-Bus | `Device.Wireless.RequestScan` was typed `a{sa{sv}}` instead of `a{sv}`. | Signature corrected and integration test covers it. |
| Low | D-Bus | Manager/Device `State` property change helper collided with `StateChanged`. | `emits_changed_signal = "false"` plus explicit signal declaration and manual emission. |
| Low | Access points | Existing APs never emitted `PropertiesChanged` when signal strength changed. | Emit per-AP changes after each scan. |
| Low | wpa backend | `SAVE_CONFIG` could rewrite `/etc/wpa_supplicant.conf` behind the administrator's back. | Removed; profiles stay in the daemon's state directory. |
| Low | Packaging | Port Makefile option conditionals used `PORT_OPTIONS` before `bsd.port.options.mk`, and assumed the target directory. | Added `bsd.port.options.mk` and an explicit `CARGO_BUILD_TARGET_DIR`. |
| Low | Wi-Fi | wpa_supplicant prints non-ASCII SSIDs as literal `\xNN` sequences; AP `Ssid` contained backslash ASCII instead of the real bytes. | `unescape_wpa` in `nm-common`, used by scan/status parsing, with tests; live scan confirmed raw UTF-8 bytes. |
| Low | wpa D-Bus | The backend only polled BSSs; it did not subscribe to the real `ScanDone` signal. | `Interface.ScanDone` listener triggers BSS refresh; mock integration test proves the chain. |
| Low | Backend selection | Auto mode fell back to the control socket without saying why, even at debug level. | Debug log now states that `fi.w1.wpa_supplicant1` was unavailable and auto mode used the control socket. |
| High | IP configuration | Association completion marked the device and ActiveConnection ACTIVATED even when no IPv4 address existed. | Device enters IP_CONFIG while DHCP runs; ActiveConnection stays ACTIVATING until an address is published; timeout degrades to CONNECTED_SITE. |
| High | DHCP integration | Wired ActivateConnection returned NotSupported, and there was no lease event path. | Wired uses the same DHCP branch; dhclient exit hooks call the internal Dhcp.Notify interface; IP4Config Gateway/DNS are updated. |
| Medium | FreeBSD paths | The implementation initially assumed `/var/run/dhclient.*.pid`; FreeBSD uses `/var/run/dhclient/dhclient.<ifname>.pid`. | Use the real path and reuse a live system dhclient instead of forking a duplicate. |
| Low | Scripts | Smoke-test GVariant numeric parsing included the type width (`<uint32 50>` became 3250), and its option loop skipped values. | Parse the last field, strip leading zeroes, and use a `while` argument loop. |
| Low | Scripts | The isolated desktop test put its D-Bus socket under a path containing non-ASCII bytes, which D-Bus rejects. | Runtime state moved to `/tmp`; screenshots/logs copied back to the repository. |

## Accepted risks / open work

- DHCP is implemented with base `dhclient` by default and optional
  `dhcpcd`. The daemon reuses a running rc.conf dhclient and otherwise starts
  a managed foreground client. A timeout leaves the device in IP_CONFIG and
  degrades the manager to CONNECTED_SITE instead of claiming an address.
- The `dhcpcd` path is experimental: there is no dhcpcd-specific lease hook in
  this MVP, so address updates rely on the normal address poll and Gateway may
  be empty. No live root-owned dhclient/hook test was performed; the staged
  install and simulated `Dhcp.Notify` path are tested.
- No IPv6, DHCPv6, DHCP option objects, SecretAgent, polkit, VPN, 802.1X, WEP,
  hidden network UI or automatic saved-profile activation.
- The optional wpa_supplicant D-Bus backend compiles but was not exercised
  against a live `fi.w1.wpa_supplicant1` service (stock FreeBSD does not have
  one).
- Live plasma-nm and GNOME panel compatibility is still untested; XFCE was
  tested in an isolated real panel.
- Full D-Bus control is restricted to `wheel`/`network`; profile owners can
  activate/deactivate their own profiles and read their own secrets. Settings
  writes still require an admin group. There is still no polkit/SecretAgent.
  The keyring is a separate 0600 file but is not encrypted at rest, so root (and
  anyone who can read root files) can recover PSKs.
- A non-`wheel`/`network` desktop user can manage own profiles but cannot edit
  system settings until added to one of those groups.
- The installed FreeBSD GNOME Control Center has no network panel and no
  `libnm` linkage; `networkmanager-shim` is a build stub. The D-Bus service
  cannot make a frontend compiled without NM support appear. Phase 4 needs a
  functional libnm/NetworkManagerQt and rebuilt desktop packages.
- A deactivate that races the very first `ObjectServer::at` registration in
  `activate` can leave an orphaned active-connection object. The window is
  narrow (immediate deactivate before the callback returns) and the object is
  removed on daemon restart. A per-device activation mutex would close it.
- The XFCE plugin creates a fresh profile UUID on each connect, so repeated
  connects to the same SSID create duplicate profiles. The daemon keeps them
  consistent, but a profile lookup would be nicer.
- The isolated desktop test clicks a fixed panel coordinate and requires a real
  `wlan0` for device enumeration; it is a test harness, not a general UI
  automation framework.
- The port skeleton has a placeholder `MAINTAINER` and no `distinfo`; run
  `make makesum` in a real ports tree. `GH_ACCOUNT` now points at
  `Nickhades/network-rs`.
- Full root installation was not executed in this environment (no sudo
  password). The same code path was validated with `--destdir`, and the
  service/smoke/desktop paths were validated on private buses.

## Low-level FreeBSD review

- netlink parsing validates message and TLV lengths before reading; the live
  RTM_GETLINK dump test runs on every FreeBSD test run.
- `AF_NETLINK = 38`, FreeBSD's length-prefixed `sockaddr_nl` and host-endian
  headers are handled explicitly; no Linux-only netlink crate is used.
- getifaddrs memory is released with `freeifaddrs`; interface addresses are
  copied before the list is freed.
- The wpa datagram client binds a unique path with mode 0600 and unlinks it on
  drop; commands are not shell-interpreted.
- No `ifconfig` output is parsed. Interface state and addresses come from
  netlink/getifaddrs; Wi-Fi control comes from wpa_supplicant.
