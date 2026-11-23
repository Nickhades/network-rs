# Wi-Fi 扫描链路

**语言：** [English](../SCAN-FLOW.md) | [简体中文](SCAN-FLOW.md)

上层和下层接口故意保持不同层次：

    桌面前端
      |  org.freedesktop.NetworkManager.Device.Wireless.RequestScan(a{sv})
      v
    nm-freebsd Core::request_scan
      |
      v
    Backend::scan
      |                                   \
      v                                    v
    wpa_supplicant D-Bus                wpa_supplicant 控制 socket
    fi.w1.wpa_supplicant1.Interface     （stock FreeBSD）
      Interface.Scan(a{sv})              SCAN
      Interface.ScanDone(b)              CTRL-EVENT-SCAN-RESULTS
      Interface.BSSs (a{oa{sv}})         SCAN_RESULTS
      BSSAdded / BSSRemoved
      |                                    |
      +----------------+-------------------+
                       v
           ScanAp -> AccessPoint D-Bus 对象
           AccessPointAdded / AccessPointRemoved
           PropertiesChanged

## 上层：NetworkManager

前端调用：

    org.freedesktop.NetworkManager.Device.Wireless.RequestScan(a{sv} options)

方法立即返回。结果稍后通过以下方式到达：

- <code>AccessPoints</code> 属性；
- <code>AccessPointAdded(o)</code> / <code>AccessPointRemoved(o)</code>；
- <code>org.freedesktop.DBus.Properties.PropertiesChanged</code>。

守护进程把每个 BSS 映射到稳定的 AccessPoint 对象：
<code>/org/freedesktop/NetworkManager/AccessPoints/&lt;counter&gt;</code>。
SSID 以原始 <code>ay</code> 字节暴露，<code>Strength</code> 为 0–100，
并带有频率、privacy 和 WPA/RSN flags。

## 下层：wpa_supplicant D-Bus

真实 D-Bus 接口是：

    fi.w1.wpa_supplicant1
      /fi/w1/wpa_supplicant1/Interfaces/<n>
        Interface.Scan(a{sv} args)
        signal Interface.ScanDone(b success)
        property Interface.BSSs a{oa{sv}}
        signal Interface.BSSAdded(o bss, a{sv} properties)
        signal Interface.BSSRemoved(o bss)

wpa_supplicant D-Bus API 中**没有 <code>GetScanResults</code> 方法**。
控制 socket 的命令叫 <code>SCAN_RESULTS</code>；D-Bus 侧对应的数据是
<code>BSSs</code> 属性。也可以使用 <code>BSSAdded</code>/<code>BSSRemoved</code>
做增量更新。

<code>backend/wpa_dbus.rs</code> 的实现：

1. 订阅 <code>ScanDone</code>；
2. <code>Backend::scan</code> 调用 <code>Interface.Scan</code>；
3. 收到 <code>ScanDone(true)</code> 后发送 <code>BackendEvent::ScanResults</code>；
4. <code>Backend::scan_results</code> 读取 <code>Interface.BSSs</code>，
   把每个字典项转换为 <code>ScanAp</code>；
5. 核心把新列表与已有 AP 对象做差异，发送
   <code>AccessPointAdded</code>/<code>AccessPointRemoved</code> 以及
   <code>PropertiesChanged</code>。

保留 10 秒一次的 BSS 轮询，仅作为漏掉 ScanDone 信号时的兜底。

## Stock FreeBSD：控制 socket

FreeBSD 基础系统的 wpa_supplicant 没有启用
<code>CONFIG_CTRL_IFACE_DBUS</code>，所以 <code>fi.w1.wpa_supplicant1</code> 不存在。
默认后端因此使用 datagram 控制 socket：

1. <code>Backend::scan</code> 发送 <code>SCAN</code>；
2. 事件 socket 收到 <code>CTRL-EVENT-SCAN-RESULTS</code>；
3. 后端发送 <code>SCAN_RESULTS</code> 并解析 tab 分隔的 BSS 列表；
4. 走同一个核心差异/AP 对象路径。

两种传输最终汇聚到同一个 <code>BackendEvent::ScanResults</code> 和
<code>Backend::scan_results</code> 抽象，所以 NetworkManager 层不需要知道使用哪种传输。

## 验证

<code>crates/nm-freebsd-daemon/tests/wpa_dbus_mock.rs</code>
在私有系统总线上启动 mock <code>fi.w1.wpa_supplicant1</code> 服务，证明：

1. 直接调用 <code>Interface.Scan</code> 会触发 <code>ScanDone</code>，
   守护进程从 <code>BSSs</code> 填充 NetworkManager AccessPoint；
2. 上层 <code>RequestScan</code> 确实到达 <code>Interface.Scan</code>
   （mock 的扫描计数器增加）。

普通端到端测试覆盖控制 socket 路径；真实硬件只读测试运行在真实 FreeBSD
<code>wlan0</code> 上。

## 当前限制

<code>RequestScan</code> 接受 options 字典，但**尚未转发定向扫描的 SSID 列表**
（<code>options["ssids"]</code>）。空字典请求完全支持；按 SSID 定向扫描是路线图项目。
