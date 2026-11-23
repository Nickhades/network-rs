# nm-freebsd

**语言：** [English](README.md) | [简体中文](README.zh-CN.md)

仓库：<https://github.com/Nickhades/network-rs>

> ### 安全警告：目前不要直接安装在多用户系统上
>
> 这个 MVP 没有 polkit，也没有 SecretAgent。root 和
> <code>wheel</code>/<code>network</code> 组成员拥有完整控制权；profile owner
> 可以创建、编辑、删除、激活/断开自己的 profile 并读取自己的密钥，
> 其他本地用户只读。整机 enable/sleep 仍需要管理员组：
> <code>pw groupmod network -m &lt;user&gt;</code>，然后注销并重新登录。
> 守护进程以 root 运行，可以修改整台机器的 Wi-Fi 配置。
> 保存的 Wi-Fi 密码放在 <code>/var/db/nm-freebsd/keyring.json</code>（权限 0600），
> 与 profile 分离，但**没有静态加密**：root 可以读取。
> 在 polkit/SecretAgent 完成前，不要在共享、多用户或存在不可信本地用户的机器上安装。

这是一个用 Rust 编写的 FreeBSD 原生 NetworkManager D-Bus 服务实现。守护进程在系统总线上拥有 <code>org.freedesktop.NetworkManager</code>，导出桌面前端已经认识的 NetworkManager 对象树。真实网络操作仍由 FreeBSD 完成：Wi-Fi 通过 wpa_supplicant，链路和事件通过 netlink，地址和 DNS 通过 getifaddrs / resolv.conf 读取。

设计原则只有一条：**让现有桌面前端直接工作；除非桌面没有可用的 NetworkManager 客户端，否则不要另写 UI。** 上游 plasma-nm/GNOME 网络面板依赖 libnm/NetworkManagerQt，而 FreeBSD 当前桌面包没有这些客户端库；XFCE 使用一个小面板插件，GNOME 使用一个 Shell 扩展，二者都是 raw D-Bus 客户端。

    桌面：plasma-nm / GNOME 网络面板 / XFCE 面板插件
                         |
              系统总线：org.freedesktop.NetworkManager
                         |
       +-----------------+------------------+
       |  zbus 对象层                        |
       |  Manager、Device(.Wireless/.Wired)、 |
       |  AccessPoint、Settings、             |
       |  Settings.Connection、Active、       |
       |  IP4Config                           |
       +-----------------+------------------+
                         |
            核心状态 + 激活状态机
                 /                       \
        wpa_supplicant                 FreeBSD netlink
        D-Bus 或 datagram 控制接口      链路 dump / 事件
                                         getifaddrs + resolv.conf

## 版本历史

- **v0.2.10 / main** — owner/admin 激活、断开与 settings 写入、owner 范围的
  <code>GetSecrets</code>、隐藏 SSID、频段/doctor 诊断，以及 DHCP 客户端冲突检测。
  本文档描述当前代码。
- 更早的版本（<code>v0.1.0</code>–<code>v0.2.8</code>）保留为 Git tag；
  历史说明见 GitHub releases。

## 当前状态

在 FreeBSD 15.1-RELEASE (amd64) 上已验证的可工作 MVP：

- 私有 D-Bus 守护进程 + 假 wpa_supplicant 控制 socket 的端到端集成测试；
- mock <code>fi.w1.wpa_supplicant1</code> 集成测试，证明
  <code>RequestScan -> Interface.Scan -> ScanDone -> BSSs -> AccessPoint</code>；
- 对真实 <code>wlan0</code> 和系统自带 wpa_supplicant 的只读验证（真实扫描返回附近 AP，真实 IPv4 地址和 DNS 通过 IP4Config 发布）；
- XFCE 面板插件作为 <code>libxfce4panel-2.0</code> 模块构建成功，在隔离 Xvfb 显示中的真实 <code>xfce4-panel</code> 里加载，并渲染状态按钮和网络菜单（见下方截图）；
- Phase 1 安全加固：<code>wheel</code>/<code>network</code> D-Bus 策略、
  按 caller uid 检查、独立的 0600 Wi-Fi keyring；
- Phase 2 DHCP：默认 base <code>dhclient</code>、可选 <code>dhcpcd</code>、
  复用系统 dhclient、exit-hook 租约事件、IP4Config Gateway、有线 DHCP
  （进程/hook 路径在测试中模拟；真实 root DHCP 测试仍为手动）；
- Phase 3 信号回归：Device/AP/IP4Config 的标准 <code>PropertiesChanged</code>、
  AP <code>Strength</code> 变化，以及以 <code>ACTIVATED</code> 结尾的 Device
  <code>StateChanged</code> 序列；
- 一键安装、依赖检查、编译测试、只读冒烟测试和隔离桌面测试脚本。

尚未在真实的 plasma-nm 或 GNOME 网络面板上做 FreeBSD 兼容测试；这是下一阶段的兼容性目标。参见[限制与路线图](#限制与路线图)。

## 已实现内容

| 区域 | 已实现 |
| --- | --- |
| Manager | 设备列表、状态/连通性、enable/sleep、权限、状态与设备信号 |
| Device | 类型、状态/原因、MTU、MAC、活动连接、IP4Config、managed/autoconnect |
| Device.Wireless | AP 列表、请求扫描、当前 AP、最近扫描时间、能力位 |
| AccessPoint | SSID（原始字节）、信号强度、频率、flags、WPA/RSN flags |
| Settings | 连接列表、读取、添加、更新、删除、重新加载、NewConnection/ConnectionRemoved |
| Settings.Connection | 读取设置/密钥、更新、未保存更新、删除、Updated/Removed |
| Connection.Active | 状态、default、设备、IP4Config、ID |
| IP4Config | 地址数据、Gateway、DNS 服务器、DNS 数据（只读） |
| Wi-Fi 后端 | 开放网络、WPA2-PSK、WPA3-SAE、扫描、状态、连接、断开 |
| DHCP | base <code>dhclient</code>（默认）、可选 <code>dhcpcd</code>、复用系统客户端、exit-hook 事件、有线和 Wi-Fi |
| 有线激活 | <code>ActivateConnection</code> DHCP 分支、链路/地址状态和 IP4Config |
| 传输层 | wpa_supplicant datagram 控制 socket（FreeBSD 默认）、可选 <code>fi.w1.wpa_supplicant1</code> D-Bus 后端 |
| 链路事件 | netlink RTM_GETLINK dump + 多播监听，getifaddrs 轮询兜底 |
| 桌面 | KDE/GNOME/XFCE/未知桌面探测与日志；XFCE 面板插件 |
| 安全 | <code>wheel</code>/<code>network</code> D-Bus 策略、按 caller uid 检查、真实 GetPermissions、独立 0600 keyring |
| 打包 | D-Bus policy、activation service、rc.d 脚本、Port skeleton |

接口签名和前端调用说明见 [docs/zh-CN/DBUS.md](docs/zh-CN/DBUS.md)；扫描链路见 [docs/zh-CN/SCAN-FLOW.md](docs/zh-CN/SCAN-FLOW.md)；代码审核见 [docs/zh-CN/CODE-REVIEW.md](docs/zh-CN/CODE-REVIEW.md)；硬件/频段支持见 [docs/zh-CN/HARDWARE.md](docs/zh-CN/HARDWARE.md)。双语文档索引见 [docs/zh-CN/README.md](docs/zh-CN/README.md)。

## FreeBSD 上的关键发现

这些结论与最初的架构草图不同，代码已按真实情况处理：

1. **系统自带的 FreeBSD wpa_supplicant 没有 D-Bus 接口。** 基础系统二进制没有启用
   <code>CONFIG_CTRL_IFACE_DBUS</code>。因此守护进程默认使用原生控制接口；该接口在 FreeBSD 上是
   **<code>SOCK_DGRAM</code> Unix socket**，客户端必须自己绑定一个路径（已用 <code>truss(1)</code> 验证）。
   对于 ports/定制构建中存在 <code>fi.w1.wpa_supplicant1</code> 的情况，仍保留 D-Bus 后端；
   <code>--backend auto</code> 会探测并回退。
2. **XFCE 绑定 crate 的实际名称是 <code>xfce-panel</code>。** 上游项目是
   [xfce-panel-plugin-rs](https://github.com/d3v2a/xfce-panel-plugin-rs)，但 crates.io 上的包是
   <code>xfce-panel</code> 和 <code>xfce-panel-sys</code>；插件使用这两个 crate。
3. **桌面探测是本地实现且有单元测试。** 面向 Linux 的
   <code>detect-desktop-environment</code> crate 只是读取两个 XDG 变量，却会额外引入依赖。
   本项目在 <code>crates/nm-common/src/desktop.rs</code> 中实现同样的
   Kde/Gnome/Xfce/Unknown 分类，语义一致并有测试。
4. **FreeBSD 14+ 可以使用 netlink。** <code>crates/nm-freebsd-daemon/src/netlink.rs</code>
   直接处理 FreeBSD 的线格式，包括与 Linux 不同、以长度字节开头的
   <code>sockaddr_nl</code>。netlink 不可用时通过 getifaddrs 轮询保持可用。
5. **dhclient PID 路径不是 <code>/var/run/dhclient.*.pid</code>。** FreeBSD 使用
   <code>/var/run/dhclient/dhclient.&lt;ifname&gt;.pid</code>；守护进程检查该文件，
   复用 rc.conf 已启动的 dhclient，避免重复 fork。租约事件来自
   <code>/etc/dhclient-exit-hooks</code>，由 <code>dhclient-script</code> 在
   BOUND/RENEW/EXPIRE 后 source。
6. **桌面前端需要功能性 NM 客户端库，而不只是 D-Bus 服务。** 本机的
   <code>gnome-control-center</code> 47.7 包没有 <code>network</code> 面板，
   也没有链接 <code>libnm</code>；<code>networkmanager-shim</code> 只是 5.9 KB
   的编译 shim。注册 <code>org.freedesktop.NetworkManager</code> 是必要条件，
   但编译时未启用 NM 的预构建前端无法在运行时补出面板。
   Phase 4 还需要功能性 <code>libnm</code>/NetworkManagerQt 和重新构建的桌面包。

## 一键安装与测试

### 预编译一键安装（不需要在 root 下编译）

release 产物已经在 <code>target/release</code> 中。在仓库根目录执行：

    sudo ./install-oneclick.sh

它会安装 daemon、XFCE 插件、<code>nm-notify-dhcp</code>、D-Bus policy、
activation service、rc.d 脚本和 dhclient hook，然后启动服务并运行冒烟测试。
只安装、不启动：

    sudo ./install-oneclick.sh --no-start --no-smoke

安装后重启面板并添加插件：

    xfce4-panel -r
    # 面板 → 添加新项目 → NetworkManager FreeBSD

卸载：

    sudo ./uninstall-oneclick.sh

安装器会备份/恢复 <code>/etc/dhclient-exit-hooks</code>。

如果 smoke 仍报 <code>org.freedesktop.NetworkManager</code> 不可达，先看
<code>sudo tail -50 /var/log/nm_freebsd.log</code>，再执行：

    sudo service dbus restart
    sudo service nm_freebsd start
    sudo service nm_freebsd status

安装器现在会在 smoke 前最多等待 30 秒让 daemon 注册总线名。
<code>service</code> 状态检查请用 root；PID 文件是 root-only。

### 构建并安装

    sudo scripts/install.sh                  # 依赖检查、构建、安装、启用/启动、冒烟
    sudo scripts/install.sh --install-deps   # 先安装缺失的 pkg 依赖
    sudo scripts/install.sh --with-plugin --install-deps  # 强制插件及其依赖
    make test                                # fmt + clippy -D warnings + 测试 + release 构建
    make desktop                             # 隔离 Xvfb/XFCE 截图测试

常用变体：

    scripts/check-deps.sh --with-plugin      # 只读依赖报告
    scripts/build-test.sh                    # 输出 build-test.log
    scripts/smoke-test.sh --scan             # 对运行中的服务做只读冒烟
    sudo scripts/install.sh --no-start       # 只安装文件，不启动
    sudo scripts/install.sh --destdir /tmp/stage --no-start --no-smoke
    scripts/install.sh --uninstall --yes     # 停止并移除已安装文件

安装器使用 <code>packaging/freebsd</code> 中的文件，会把已有的 policy、service、rc 和
dhclient hook 文件备份为 <code>*.nm-freebsd.bak</code>，安装
<code>nm-notify-dhcp</code> 并追加 DHCP exit-hook source 行，启用 rc 服务，通过
<code>org.freedesktop.DBus.ReloadConfig</code> 应用策略，然后运行冒烟测试。
如果 Rust 来自当前用户的 rustup，安装器会为 root 构建推导
<code>CARGO_HOME</code>/<code>RUSTUP_HOME</code>；开发时最好由用户自己构建并使用
<code>--no-build</code>。除非执行 <code>--uninstall --purge</code>，否则不会删除
<code>/var/db/nm-freebsd</code>。

## 桌面渲染测试

<code>scripts/desktop-test.sh</code> 会创建一个一次性环境：空闲 display 上的 Xvfb、
私有 session bus、私有 system bus、假 wpa_supplicant、真实守护进程和真实 XFCE panel。
它把插件安装到临时 HOME，写一个单面板 xfconf 配置，用 XTest 点击插件，并分别截取面板和菜单。
当前 <code>:0</code> 会话和面板配置不会被修改。

![XFCE 面板插件](artifacts/desktop-test/nm-freebsd-panel.png)

![网络菜单](artifacts/desktop-test/nm-freebsd-menu.png)

截图中是 <code>xfce4-panel</code> 4.20.8 加载的真实插件：按钮显示
“Wi-Fi: Cafe WiFi”；菜单列出假 AP 的强度、锁定状态以及 Rescan/Disconnect。
日志写入 <code>artifacts/desktop-test/logs/</code>。该测试需要
<code>Xvfb</code>、<code>xfce4-panel</code>、<code>xfce4-screenshooter</code>、
XTest 开发文件（<code>pkg install libXtst</code>），以及一个真实的
<code>wlan0</code> 用于设备枚举。

## CLI

<code>nm-freebsd-cli</code> 是同一套 NetworkManager D-Bus 契约上的瘦客户端，
不直接打开 wpa_supplicant socket，也不自己运行 DHCP。

    nm-freebsd-cli status
    nm-freebsd-cli devices
    nm-freebsd-cli connections
    nm-freebsd-cli wifi list --scan
    nm-freebsd-cli doctor
    nm-freebsd-cli wifi connect "云涧" --password '...'
    nm-freebsd-cli wifi connect "隐藏网络" --password '...' --hidden
    nm-freebsd-cli disconnect

如果 daemon 较旧、没有 <code>AddAndActivateConnection2</code>，CLI 会回退到
<code>AddAndActivateConnection</code>。

## GNOME Shell 扩展

FreeBSD 的 GNOME 包没有 libnm，因此只注册 D-Bus 名称不会让原生 GNOME 网络菜单出现。
<code>gnome-extension/</code> 提供一个 raw D-Bus 的 Shell 扩展（GNOME 45+ ESM）：

    scripts/install-gnome-extension.sh
    # 或 make gnome-extension

它只安装到当前用户
<code>~/.local/share/gnome-shell/extensions/nm-freebsd@nickhades.github.io</code>，
并把 UUID 加入 <code>org.gnome.shell enabled-extensions</code>。重新登录，
或在 X11 下按 <code>Alt+F2</code> 输入 <code>r</code>。扩展调用
<code>GetDevices</code>、<code>GetAllAccessPoints</code>、
<code>AddAndActivateConnection</code>，订阅 <code>StateChanged</code>、
<code>AccessPointAdded/Removed</code> 和 AP <code>PropertiesChanged</code>；
没有轮询。

![GNOME Shell 扩展菜单](artifacts/gnome-extension-menu.png)

截图来自隔离的 Xvfb + nested GNOME Shell 47.10、私有 session bus 和真实
system-bus daemon；扩展状态为 <code>enabled=true</code>、<code>error=""</code>。

## 构建

要求：

- FreeBSD 15.x（14.x 应可工作；实时事件需要 netlink，不可用时守护进程会回退轮询）；
- Rust 1.95 或更新版本（XFCE 绑定要求 edition 2024 / 1.95）；
- 插件需要 <code>pkgconf</code>、GTK 3、libxfce4panel 2.0 开发文件；
- 运行时需要 D-Bus 系统总线和 wpa_supplicant。

    cargo build --release -p nm-freebsd-daemon
    cargo build --release -p nm-dhcp-notify
    cargo build --release -p nm-xfce-plugin   # 可选，需要 XFCE 开发库
    cargo test --workspace

守护进程二进制是 <code>target/release/nm-freebsd-daemon</code>；DHCP hook 桥是
<code>target/release/nm-notify-dhcp</code>；插件是
<code>target/release/libnm_freebsd_xfce.so</code>，生成的 desktop 文件位于
<code>target/release/build/nm-xfce-plugin-*/out/nm_freebsd_xfce.desktop</code>。

## 运行

守护进程必须以 root 运行：FreeBSD 的 wpa_supplicant 控制 socket 是
<code>root:wheel</code>、权限 0770；netlink 链路配置也需要权限。

快速私有总线冒烟测试（不修改系统总线策略）：

    # 终端 1：私有总线
    dbus-daemon --config-file=/path/to/permissive-system.conf --nofork

    # 终端 2
    DBUS_SYSTEM_BUS_ADDRESS=unix:path=/path/to/bus \
      sudo -E target/release/nm-freebsd-daemon \
        --backend wpa-ctrl --wpa-ctrl-dir /var/run/wpa_supplicant

生产环境安装 <code>packaging/freebsd</code> 中的文件：

    install -m 755 target/release/nm-freebsd-daemon /usr/local/bin/
    install -d /usr/local/libexec
    install -m 755 target/release/nm-notify-dhcp /usr/local/libexec/
    sed 's|%%PREFIX%%|/usr/local|g' packaging/freebsd/libexec/nm-freebsd-dhcp-hook.in \
      > /usr/local/libexec/nm-freebsd-dhcp-hook
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

一键安装器还会安装 <code>nm-notify-dhcp</code> 并追加 DHCP source 行。
手工安装时需要自行添加：

    printf '\n. /usr/local/libexec/nm-freebsd-dhcp-hook\n' >> /etc/dhclient-exit-hooks

Port skeleton 位于 <code>packaging/freebsd/ports/net-mgmt/nm-freebsd</code>，参见
[packaging/freebsd/README.zh-CN.md](packaging/freebsd/README.zh-CN.md)。
Port 的 <code>cargo-crates.mk</code> 由
[tools/gen-port-crates.py](tools/gen-port-crates.py) 从 <code>Cargo.lock</code> 生成。

### 守护进程选项

    --backend auto|wpa-ctrl|wpa-dbus   wpa_supplicant 传输方式（默认 auto）
    --dhcp-backend auto|dhclient|dhcpcd  DHCP 客户端（默认 auto，优先 base dhclient）
    --dhcp-timeout SECONDS             等待租约后降级到 CONNECTED_SITE 的秒数（默认 30）
    --wpa-ctrl-dir PATH                默认 /var/run/wpa_supplicant
    --state-dir PATH                   配置存储，默认 /var/db/nm-freebsd
    --wifi-interface NAME              可重复；未指定时自动探测 wlan*
    --no-wired                         不暴露以太网接口
    --bus system|session               默认 system
    --bus-address ADDRESS              显式 D-Bus 地址（jail/测试）
    --desktop auto|kde|gnome|xfce|unknown
    --no-initial-scan                  跳过启动时的 Wi-Fi 扫描
    --log-level FILTER                 也支持环境变量 NM_FREEBSD_LOG

## 桌面行为

- **KDE / GNOME：** 守护进程不启动任何 UI。plasma-nm 或 GNOME 面板负责用户体验，并通过
  D-Bus 与守护进程通信。
- **XFCE：** 使用启用了 XFCE 选项的 Port 安装后，面板的“添加新项目”中会出现
  “NetworkManager FreeBSD”。插件只需要 D-Bus；它显示当前状态、按需扫描、列出网络、
  询问 WPA 密码并调用 <code>AddAndActivateConnection</code>。
- **未知桌面：** 只提供 D-Bus；守护进程会记录提示。无头环境可设置
  <code>XDG_CURRENT_DESKTOP</code> 或使用 <code>--desktop</code>。

XFCE 插件是会话级的，守护进程是系统级的。插件每次请求都会新建系统总线连接，
因此重启守护进程不需要重启面板。

## 配置存储

配置是 <code>/var/db/nm-freebsd/connections/*.json</code> 下的小型 JSON 文件
（目录 0700，文件 0600）。只持久化守护进程实际能应用的字段：UUID、id、类型、
SSID（字节数组）、密钥管理方式、自动连接和时间戳。密码单独存放在
<code>/var/db/nm-freebsd/keyring.json</code>（0600），以 profile UUID 为键；
旧 profile 中的明文 <code>psk</code> 会在启动时迁移并重写为不含密码的格式。
D-Bus settings 字典由这些字段重建，密钥不会写入日志。

## 安全

root 和 <code>wheel</code>/<code>network</code> 组成员拥有完整方法访问权；
守护进程还会对每个操作独立检查 caller uid。非管理员用户可以：

- 激活 <code>owner_uid</code> 等于自己的 profile；
- 断开自己拥有的活动连接；
- 对自己拥有的活动连接调用 <code>Device.Disconnect()</code>；
- 读取自己 profile 的 <code>GetSecrets()</code>；
- 调用 <code>AddAndActivateConnection[2]</code>，创建归自己所有的 profile 并激活；
- 读取 Manager/Device/AP 属性、列出不含密钥的 settings、发起扫描。

profile owner 或管理员可以创建、<code>Update</code>、<code>Delete</code>
自己的 profile；<code>Enable</code>、<code>Sleep</code> 和整机操作仍要求
<code>wheel</code>/<code>network</code>。没有 <code>owner_uid</code> 的旧 profile
仍仅限管理员。D-Bus 策略允许 owner 路径进入 daemon，由 daemon 做 owner/admin 判定。

目前仍然没有 polkit 和 SecretAgent。<code>GetPermissions</code> 只回答整机控制权限，
不表达 per-profile ownership。管理员可以授予完整控制权：

    pw groupmod network -m <user>

然后注销并重新登录。keyring 是独立的 0600 文件，但**没有静态加密**，
root 可以读取。不要把系统总线暴露给不可信机器。应把
<code>/var/db/nm-freebsd</code> 视为敏感目录。

## 限制与路线图

本 MVP 明确不包含：

- DHCP 通过 <code>dhclient</code>（默认）或可选 <code>dhcpcd</code> 实现。
  没有系统实例时启动受管客户端，否则复用现有实例。exit hooks 上报
  BOUND/RENEW/REBIND/EXPIRE。租约超时（默认 30 秒）后设备保持 IP_CONFIG，
  Manager 降级到 CONNECTED_SITE，而不是谎报可用 IP。
- 不支持 IPv6（<code>IP6Config</code> 为 <code>/</code>）、DHCPv6、DHCP option 对象、SecretAgent、
  polkit、VPN、企业级 802.1X、WEP。隐藏 SSID 已支持 CLI/profile，但 XFCE/GNOME
  面板还没有隐藏网络输入 UI。
- 不自动激活已保存配置。wpa_supplicant 自己配置的网络仍可能关联，守护进程会报告该状态。
- 守护进程启动前已经连上的 Wi-Fi 会在启动时被接管为
  <code>ActiveConnection</code>，并绑定匹配的 profile 和现有 IPv4 配置；
  不需要前端再触发一次激活。
- 只有非空扫描结果不再包含某个 AP 时才移除 AP；空扫描结果保留旧列表以避免界面闪烁。
- XFCE 插件由 NetworkManager 标准信号驱动，保留 30 秒兜底轮询；
  守护进程重启后会重连信号监听。
- 真实 plasma-nm、GNOME 面板兼容测试仍待完成。本机 GNOME Control Center
  编译时没有 network 面板，现有 <code>networkmanager-shim</code> 也是空壳；
  要出现面板还需要功能性 libnm/重新构建的桌面包。

## 仓库结构

    crates/nm-common/          常量、桌面探测、SSID/安全解析辅助
    crates/nm-freebsd-daemon/  守护进程、D-Bus 对象、wpa_ctrl/wpa_dbus、netlink、DHCP
      tests/integration.rs     私有总线 + 假 wpa/客户端端到端测试
      tests/wpa_dbus_mock.rs   mock fi.w1.wpa_supplicant1 扫描链路测试
    crates/nm-dhcp-notify/     dhclient-exit-hooks -> Dhcp.Notify 桥
    crates/nm-freebsd-cli/     瘦 NM D-Bus CLI
    crates/nm-xfce-plugin/     XFCE 面板插件（cdylib）
    scripts/                   安装、依赖、编译测试、冒烟、桌面测试
    packaging/freebsd/         policy、activation service、rc.d、Port skeleton
    tools/                     假 wpa 服务、Port crate 列表生成器
    artifacts/desktop-test/    面板/菜单截图
    docs/                      英文规范文档（含硬件/频段矩阵）
    docs/zh-CN/                中文镜像文档
    install-oneclick.sh        预编译 root 安装器（无需 Rust 构建）
    uninstall-oneclick.sh      卸载入口
    Makefile                   脚本快捷入口

## 许可证

BSD-2-Clause。参见 [LICENSE](LICENSE)。
