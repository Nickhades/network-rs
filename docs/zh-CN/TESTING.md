# 测试

**语言：** [English](../TESTING.md) | [简体中文](TESTING.md)

## 自动化测试

统一入口：

    scripts/check-deps.sh --with-plugin
    scripts/build-test.sh
    scripts/smoke-test.sh --scan
    scripts/desktop-test.sh

<code>build-test.sh</code> 依次运行 fmt、clippy、整个 workspace 的测试和 release 构建，
日志写入 <code>build-test.log</code>。原始命令是：

    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all --check

测试套件包含：

- <code>nm-common</code>：桌面探测、wpa 安全 flags 解析、SSID 字节/hex/转义辅助函数；
- daemon 单元测试：wpa 扫描/状态解析、netlink 消息解析（合成消息和真实链路 dump）、
  getifaddrs/ifindex 一致性、配置持久化、settings round-trip、UUID 路径穿越拒绝、CLI 解析；
- 一个插件 v4 UUID 形状测试；
- 集成测试 <code>mvp_end_to_end</code>（私有总线、假 datagram wpa_supplicant、
  真实守护进程、zbus 客户端），位于
  <code>crates/nm-freebsd-daemon/tests/integration.rs</code>；
- 集成测试 <code>request_scan_uses_wpa_dbus_scan_done</code>（mock
  <code>fi.w1.wpa_supplicant1</code> 服务），位于
  <code>crates/nm-freebsd-daemon/tests/wpa_dbus_mock.rs</code>：
  证明上层 <code>RequestScan</code> 到达 <code>Interface.Scan</code>，
  且 <code>ScanDone</code> 信号能从 <code>BSSs</code> 刷新 NetworkManager AP；
  详见 [SCAN-FLOW.md](SCAN-FLOW.md)。

## 集成测试的工作方式

<code>mvp_end_to_end</code> 自包含：

1. 用每测试独立的宽松配置启动私有 <code>dbus-daemon</code>；
2. 启动一个 Unix datagram 的假 wpa_supplicant；
3. 通过 <code>daemon::start</code> 用显式总线地址和临时状态目录启动真实守护进程；
4. 用真实 zbus 客户端检查：
   - Wi-Fi 设备发现；
   - <code>RequestScan</code> 和 <code>GetAllAccessPoints</code>，包括 SSID、强度、
     privacy flags；
   - <code>AddAndActivateConnection</code>；
   - wpa_supplicant 收到 <code>ADD_NETWORK</code>、hex SSID、<code>PSK</code>、
     <code>SELECT_NETWORK</code>；
   - 活动连接状态 2、设备状态 100、Manager 状态 60；
   - 发出 <code>Manager.StateChanged</code>；
   - <code>Settings.ListConnections</code>/<code>GetConnection</code> round-trip；
   - <code>DeactivateConnection</code> 后设备回到 30、Manager 回到 20。

它不会修改宿主机网络。没有 <code>dbus-daemon</code> 时会优雅跳过。

<code>request_scan_uses_wpa_dbus_scan_done</code> 使用真实 netlink 暴露的
<code>wlan0</code>，在私有总线上运行 mock wpa D-Bus 服务，不触碰真实关联。

## 真实硬件只读验证

已在 FreeBSD 15.1 + 真实 <code>wlan0</code> + 基础系统 wpa_supplicant 上验证，
不改变当前关联：

- 守护进程连接到真实控制 socket（datagram，不是 stream）；
- 初始 STATUS 得到 <code>Device.State = 100</code>；
- IP4Config 发布真实地址（192.168.2.225/24）和 resolv.conf 名称服务器；
- 真实扫描返回 9–10 个附近 AP，包含原始 SSID 字节、强度和 privacy flags；
- 扫描后 <code>ActiveAccessPoint</code> 解析到当前 BSSID 对应的 AP 和 SSID。

只读验证方式：

    DBUS_SYSTEM_BUS_ADDRESS=unix:path=/tmp/nmtest/bus \
      ./target/debug/nm-freebsd-daemon \
        --backend wpa-ctrl \
        --wpa-ctrl-dir /var/run/wpa_supplicant \
        --state-dir /tmp/nm-state \
        --wifi-interface wlan0 \
        --no-initial-scan --log-level debug

然后用 <code>gdbus</code> 读取属性：

    gdbus call --system --dest org.freedesktop.NetworkManager \
      --object-path /org/freedesktop/NetworkManager \
      --method org.freedesktop.NetworkManager.GetDevices

通过 <code>org.freedesktop.DBus.Properties.GetAll</code> 读取
<code>Device.State</code>、<code>Device.Wireless.ActiveAccessPoint</code>、
AP 属性和 <code>Device.Ip4Config</code>。

## 桌面渲染测试

<code>scripts/desktop-test.sh</code> 不触碰当前 <code>:0</code> 会话，
端到端验证 XFCE 集成：

1. 在空闲 display 上启动 Xvfb；
2. 用 <code>dbus-launch</code> 和 <code>xfconfd</code> 启动私有 session bus；
3. 启动私有 system bus、假 datagram wpa_supplicant 和真实守护进程；
4. 把真实插件安装到临时 HOME，写入单面板 xfconf 配置；
5. 启动真实 <code>xfce4-panel</code>，等待窗口，截取面板；
6. 用 XTest helper 点击插件，截取网络菜单。

FreeBSD 15.1 上的结果：

![面板](../../artifacts/desktop-test/nm-freebsd-panel.png)

![菜单](../../artifacts/desktop-test/nm-freebsd-menu.png)

按钮显示 <code>Wi-Fi: Cafe WiFi</code>；菜单列出
<code>Cafe WiFi 100% locked</code>、<code>Lab 5G 74% locked</code>、
<code>Open Net 46%</code>、Rescan 和 Disconnect。
假 wpa_supplicant 位于 <code>tools/fake-wpa.py</code>；日志复制到
<code>artifacts/desktop-test/logs/</code>。

该测试需要 <code>Xvfb</code>、<code>xfce4-panel</code>、
<code>xfce4-screenshooter</code>、XTest 开发文件
（<code>pkg install libXtst</code>）以及真实 <code>wlan0</code> 用于设备枚举。
它不会修改真实 Wi-Fi 关联。

## 手动前端检查

### Introspection

    gdbus introspect --system --dest org.freedesktop.NetworkManager \
      --object-path /org/freedesktop/NetworkManager
    gdbus introspect --system --dest org.freedesktop.NetworkManager \
      --object-path /org/freedesktop/NetworkManager/Settings

确认 <code>StateChanged(u)</code>、<code>DeviceAdded(o)</code>、
<code>GetDevices() -&gt; ao</code>、
<code>AddAndActivateConnection(a{sa{sv}},o,o) -&gt; oo</code> 和
<code>State</code> 属性。

### Settings CRUD

使用 Python dbus 或小型 zbus 客户端；<code>a{sa{sv}}</code> 的构造参考集成测试。检查：

- <code>AddConnection</code> 返回对象路径并写入 0600 JSON 文件；
- <code>GetConnection</code> 返回 <code>connection</code>、<code>802-11-wireless</code>、
  <code>ipv4</code>、<code>ipv6</code>；
- <code>GetSecrets</code> 只在 security 子字典返回 psk；
- <code>Update</code> 发出 <code>Updated</code>；
- <code>Delete</code> 发出 <code>Removed</code> 并删除文件。

### XFCE 插件

    cargo build --release -p nm-xfce-plugin
    nm -D target/release/libnm_freebsd_xfce.so | grep xfce_panel_module_init
    readelf -d target/release/libnm_freebsd_xfce.so | grep -E 'xfce4panel|gtk-3'

按 [packaging/freebsd/README.zh-CN.md](../../packaging/freebsd/README.zh-CN.md)
安装 <code>.so</code> 和生成的 <code>.desktop</code>，重启 <code>xfce4-panel</code>，
在“添加新项目”里添加 “NetworkManager FreeBSD”。菜单应列出真实 AP，
连接动作走与集成测试相同的 D-Bus 路径。没有可用面板时，编译和符号检查是自动化兜底。

### 真实连接测试（破坏性，使用备用接口）

仅在测试机或备用 wlan clone 上操作：

1. 保存当前网络配置。
2. 用 <code>--backend wpa-ctrl</code> 启动守护进程。
3. 用已知 SSID/PSK 调用 <code>AddAndActivateConnection</code>。
4. 确认 <code>wpa_cli -i &lt;if&gt; status</code> 到达 <code>COMPLETED</code>。
5. 确认 <code>Device.State = 100</code>、活动连接状态 2、Manager 状态 60。
6. 如果该接口有 DHCP 客户端，确认 <code>IP4Config.AddressData</code> 有值；
   守护进程本身不启动 DHCP。
7. <code>DeactivateConnection</code>；确认回到 <code>DISCONNECTED</code>，
   且 wpa_supplicant 不再把该网络作为 current。

不要在无法恢复的生产上行链路上执行。

## 一键安装验证

安装脚本以两种方式验证：

    # 普通用户 staged 安装，不修改服务
    scripts/install.sh --destdir /tmp/stage --no-build --no-start --no-smoke
    scripts/install.sh --destdir /tmp/stage --uninstall --yes

    # 只展示系统安装计划，不修改系统
    scripts/install.sh --dry-run --no-build --no-start --no-smoke

当前环境没有 root/sudo 密码，因此没有执行完整 root 安装；staged 树验证了精确的二进制、
policy、activation service、rc script 和插件路径。

## 故障排查

- **<code>Failed to own org.freedesktop.NetworkManager</code>**：另一个守护进程拥有该名称，
  或 D-Bus 策略未重新加载。安装 policy 后执行 <code>service dbus restart</code>，
  并查看 <code>/var/log/messages</code>。
- **<code>Protocol wrong type for socket</code>**：对 wpa_supplicant 使用了 stream socket。
  后端使用 datagram；定制构建可用 <code>truss</code> 验证。
- **<code>/var/run/wpa_supplicant/wlan0</code> 权限拒绝**：守护进程需要 root，
  或用户属于 <code>wheel</code> 组。Port 的 rc 脚本以 root 运行。
- **扫描返回 <code>FAIL-BUSY</code>**：wpa_supplicant 正在扫描。
  <code>RequestScan</code> 把这种情况视为成功。
- **已连接但没有 IP**：没有 DHCP 客户端时的预期行为；参见
  [FREEBSD-NOTES.md](FREEBSD-NOTES.md) 和 README 的限制。
- **没有 AP**：检查 <code>wpa_cli -i wlan0 scan_results</code>；
  守护进程使用相同数据。新扫描可能需要几秒。
- **插件显示 “Wi-Fi unavailable”**：守护进程未运行、总线策略阻止了该用户，
  或名称被其他进程拥有。悬停查看错误；插件每 5 秒重试。

## CI 说明

集成测试需要 <code>dbus-daemon</code> 和可写的 <code>/tmp</code>。
FreeBSD 上的单元测试包含真实 netlink dump，必要时以
<code>#[cfg(target_os = "freebsd")]</code> 限制。XFCE 插件需要 GTK 3 和
libxfce4panel 开发文件，因此无头 CI 可以只构建 <code>nm-freebsd-daemon</code>，
或提供 XFCE SDK。
