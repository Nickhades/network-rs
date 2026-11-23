# Code review

**Languages:** [English](CODE-REVIEW.md) | [简体中文](zh-CN/CODE-REVIEW.md)

## 中文摘要

本轮审核覆盖 Rust 守护进程/插件、安装与测试脚本、FreeBSD 打包。重点修复了
wpa_supplicant 控制接口的 datagram 类型、D-Bus 方法内自死锁、非 ASCII SSID
的 wpa 转义、配置文件 UUID 路径穿越、插件读取 `/dev/urandom` 阻塞、激活/断开
竞态、首次激活 IP4Config 缺失和有线接口状态等问题。剩余的主要产品级缺口是
DHCP/地址路由配置、SecretAgent/polkit、IPv6、有线激活，以及尚未在真实
plasma-nm/GNOME 面板上验证。自动测试、私有总线冒烟测试、真实 wpa 只读扫描和
隔离 XFCE 面板渲染均已通过。

Scope: the Rust workspace, shell install/test tooling and FreeBSD packaging in
this repository. This is a working review, not a formal security audit.

## Verification at review time

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`: 16 common + 18 daemon + 1 plugin + 2 end-to-end
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
| Low | Scripts | Smoke-test GVariant numeric parsing included the type width (`<uint32 50>` became 3250), and its option loop skipped values. | Parse the last field, strip leading zeroes, and use a `while` argument loop. |
| Low | Scripts | The isolated desktop test put its D-Bus socket under a path containing non-ASCII bytes, which D-Bus rejects. | Runtime state moved to `/tmp`; screenshots/logs copied back to the repository. |

## Accepted risks / open work

- **No DHCP or address/route configuration.** Association is performed, but the
  daemon only publishes addresses that FreeBSD already has. A system without an
  existing DHCP client will have no IPv4 address after connecting.
- **Wired activation is read-only.** Ethernet links and addresses are exposed,
  but `ActivateConnection` returns `NotSupported`.
- No IPv6, DHCP option objects, SecretAgent, polkit, VPN, 802.1X, WEP, hidden
  network UI or automatic saved-profile activation.
- The optional wpa_supplicant D-Bus backend compiles but was not exercised
  against a live `fi.w1.wpa_supplicant1` service (stock FreeBSD does not have
  one).
- Live plasma-nm and GNOME panel compatibility is still untested; XFCE was
  tested in an isolated real panel.
- The D-Bus policy allows all local users to call the root daemon. There is no
  authorization layer; tighten the policy for multi-user machines.
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
