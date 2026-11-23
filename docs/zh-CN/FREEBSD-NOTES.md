# FreeBSD 实现备注

**语言：** [English](../FREEBSD-NOTES.md) | [简体中文](FREEBSD-NOTES.md)

## stock FreeBSD 的 wpa_supplicant 没有 D-Bus

最初的架构假设存在 <code>fi.w1.wpa_supplicant1</code>。FreeBSD 15.1 基础系统的
wpa_supplicant v2.11 没有启用 <code>CONFIG_CTRL_IFACE_DBUS</code>：
<code>strings</code> 找不到 <code>fi.w1</code> 服务名。因此守护进程默认使用控制 socket，
同时保留 D-Bus 后端供 ports 构建使用。

D-Bus 后端（仅当 <code>fi.w1.wpa_supplicant1</code> 存在时）遵循真实 API：
<code>Interface.Scan</code> -> <code>ScanDone</code> -> <code>BSSs</code>，
不是 <code>GetScanResults</code> 方法。mock 服务测试覆盖了这条路径，详见
[SCAN-FLOW.md](SCAN-FLOW.md)。

控制接口也不是 stream socket。用 <code>truss(1)</code> 跟踪 <code>wpa_cli</code> 可以看到：

    socket(PF_LOCAL,SOCK_DGRAM,0) = 3
    bind(3,{ AF_UNIX "/tmp/wpa_ctrl_15764-1" },106) = 0
    connect(3,{ AF_UNIX "/var/run/wpa_supplicant/wlan0" },106) = 0
    sendto(3,"STATUS",6,0,NULL,0) = 6
    recvfrom(3,"bssid=04:8b:..."...,4095,0,NULL,0x0) = 249

<code>backend/wpa_ctrl.rs</code> 中实现的结论：

- 客户端 socket 是 <code>SOCK_DGRAM</code>，在 <code>std::env::temp_dir()</code>
  下绑定一个唯一的私有路径（权限 0600）；
- 命令末尾不带换行；
- 每个回复是一个 datagram，不需要做流式分帧；
- <code>ATTACH</code> 后保持客户端 socket，事件作为后续 datagram 到达；
- 客户端路径在 drop 时删除。

第一版使用 <code>tokio::net::UnixStream</code>，在 stream 假服务器上通过了测试，
但真实硬件立即返回 <code>EPROTOTYPE</code>，暴露了问题。集成测试里的假服务器现在也使用
<code>UnixDatagram</code>。

## netlink

FreeBSD 14+ 提供 netlink(4)。守护进程用它做：

- 启动时的 <code>RTM_GETLINK</code> dump；
- <code>RTMGRP_LINK</code> / <code>RTMGRP_IPV4_IFADDR</code> 多播通知。

FreeBSD 的线格式与 Linux 类似但不完全相同。尤其是
<code>struct sockaddr_nl</code> 以 <code>nl_len</code> 开头，<code>AF_NETLINK</code> 为 38。
常量取自 <code>/usr/include/netlink</code>。实现直接解析 TLV，不依赖 Linux-only crate。

如果监听 socket 无法创建，守护进程回退到 5 秒一次的 getifaddrs 轮询。
接口状态、MAC、MTU 和 IPv4 地址仍然可用。IPv4 地址和 DNS 服务器从 getifaddrs 和
<code>/etc/resolv.conf</code> 读取；守护进程不运行 DHCP 客户端。

## D-Bus 策略与激活

文件：

- <code>packaging/freebsd/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf</code>
- <code>packaging/freebsd/etc/dbus-1/system-services/org.freedesktop.NetworkManager.service.in</code>
- <code>packaging/freebsd/etc/rc.d/nm_freebsd</code>

FreeBSD 上的安装路径：

    /usr/local/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf
    /usr/local/share/dbus-1/system-services/org.freedesktop.NetworkManager.service
    /usr/local/etc/rc.d/nm_freebsd

策略允许 root 拥有名称，所有本地用户向该服务发送/接收消息。service 文件允许总线按需以 root
启动守护进程。rc.d 脚本使用 <code>/usr/sbin/daemon -p</code> 后台化，并写
<code>/var/run/nm_freebsd.pid</code>。

FreeBSD 的 dbus rc 脚本没有 reload 命令，因此 <code>scripts/install.sh</code>
在安装策略后通过 <code>dbus-send</code> 调用
<code>org.freedesktop.DBus.ReloadConfig</code>。这会重新读取策略和 service 目录，
不会断开桌面客户端。如果本地修改的总线配置忽略了 reload，安装器会打印
<code>service dbus restart</code> 兜底提示。

守护进程需要 root：wpa_supplicant socket 和接口操作都需要权限。本 MVP 没有特权分离辅助进程。

## XFCE 面板插件 ABI

上游项目 [xfce-panel-plugin-rs](https://github.com/d3v2a/xfce-panel-plugin-rs)
发布 <code>xfce-panel</code> 和 <code>xfce-panel-sys</code> crate。插件：

- 是名为 <code>libnm_freebsd_xfce.so</code> 的 <code>cdylib</code>；
- 导出 <code>xfce_panel_module_init</code>（已用 <code>nm -D</code> 验证）；
- 链接 <code>libxfce4panel-2.0.so.4</code>、GTK 3、GLib；
- 在 <code>build.rs</code> 中通过 crate 的 <code>build</code> feature 生成
  <code>.desktop</code>。

安装路径：

    /usr/local/lib/xfce4/panel/plugins/libnm_freebsd_xfce.so
    /usr/local/share/xfce4/panel/plugins/nm_freebsd_xfce.desktop

面板会扫描这些目录。插件运行在面板进程内，因此不要在插件已加载时原地覆盖
<code>.so</code>；应安装到临时名再 rename，然后重启 <code>xfce4-panel</code>。

插件的 D-Bus worker 与守护进程生命周期独立。每次请求都新建连接，所以守护进程重启后，
下一次 5 秒刷新或用户操作会自动恢复。

## Port

<code>packaging/freebsd/ports/net-mgmt/nm-freebsd</code> 是 Port skeleton。
在真实 ports tree 中构建前：

- 把 <code>MAINTAINER</code> 改成真实地址；<code>GH_ACCOUNT</code> 已指向
  <code>Nickhades/network-rs</code>；
- 运行 <code>make makesum</code> 生成 <code>distinfo</code>
  （skeleton 故意不包含伪造的源码校验和）；
- 用
  <code>python3 tools/gen-port-crates.py &gt; packaging/freebsd/ports/net-mgmt/nm-freebsd/cargo-crates.mk</code>
  重新生成 crate 列表。

XFCE 选项会引入 GTK 3 和 <code>sysutils/xfce4-panel</code>；无头系统可以关闭。
<code>wpa_supplicant</code> 属于基础系统，不需要 Port 依赖。
