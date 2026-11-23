# D-Bus 接口

**语言：** [English](../DBUS.md) | [简体中文](DBUS.md)

服务名：<code>org.freedesktop.NetworkManager</code>（系统总线）

以下对象路径均由 zbus 提供。类型是 introspection 报告的 D-Bus 签名。

## Manager

路径：<code>/org/freedesktop/NetworkManager</code>

属性：

| 名称 | 类型 | 说明 |
| --- | --- | --- |
| Version | s | <code>1.46.0</code> |
| State | u | NMState；外部已建立链路时可为 50 |
| Startup | b | false |
| NetworkingEnabled | b | |
| WirelessEnabled | b | 可读写 |
| WirelessHardwareEnabled | b | 存在 Wi-Fi 设备时为 true |
| Connectivity | u | 4 full、3 limited、1 none、0 unknown |
| Devices | ao | 所有暴露的设备 |
| ActiveConnections | ao | |
| PrimaryConnection | o | 无时为 <code>/</code> |
| ActivatingConnection | o | 无时为 <code>/</code> |
| Metered | u | 0 unknown |
| GlobalDnsConfiguration | a{sv} | 空 |

方法：

| 方法 | 签名 |
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

信号：<code>StateChanged(u)</code>、<code>DeviceAdded(o)</code>、
<code>DeviceRemoved(o)</code>，以及 <code>org.freedesktop.DBus.Properties</code>
上的 <code>PropertiesChanged</code>。

## Device

路径：<code>/org/freedesktop/NetworkManager/Devices/&lt;ifindex&gt;</code>

属性包括 <code>Udi</code>、<code>Interface</code>、<code>IpInterface</code>、
<code>Driver</code>、<code>DriverVersion</code>、<code>FirmwareVersion</code>、
<code>Capabilities</code>、<code>State</code>、<code>StateReason</code>（<code>(uu)</code>）、
<code>ActiveConnection</code>、<code>AvailableConnections</code>、
<code>Autoconnect</code>、<code>Managed</code>、<code>DeviceType</code>、
<code>Dhcp4Config</code>、<code>Ip4Config</code>、<code>Ip6Config</code>、
<code>Mtu</code>，以及废弃的 <code>Ip4Address</code>。

方法：<code>Reapply(a{sa{sv}}, t, u)</code> 和 <code>Delete()</code> 返回
<code>NotSupported</code>；它们存在是为了让前端可以 introspection 并优雅失败。

信号：<code>StateChanged(u new_state, u old_state, u reason)</code>。

### Device.Wireless

属性：<code>HwAddress</code>、<code>PermHwAddress</code>、<code>Mode</code>、
<code>Bitrate</code>、<code>AccessPoints</code> (ao)、<code>ActiveAccessPoint</code>、
<code>WirelessCapabilities</code>、<code>LastScan</code> (x)。

方法：

- <code>GetAccessPoints() -> ao</code>（废弃）
- <code>GetAllAccessPoints() -> ao</code>
- <code>RequestScan(a{sv} options)</code>

信号：<code>AccessPointAdded(o)</code>、<code>AccessPointRemoved(o)</code>。

下层扫描映射（包括为什么 wpa_supplicant D-Bus 使用 <code>BSSs</code> 而不是
<code>GetScanResults</code> 方法）见 [SCAN-FLOW.md](SCAN-FLOW.md)。

### Device.Wired

<code>HwAddress</code>、<code>PermHwAddress</code>、<code>Speed</code>、
<code>Carrier</code>。本 MVP 中有线设备是只读的。

## AccessPoint

路径：<code>/org/freedesktop/NetworkManager/AccessPoints/&lt;counter&gt;</code>

<code>Flags</code> (u)、<code>WpaFlags</code> (u)、<code>RsnFlags</code> (u)、
<code>Ssid</code>（ay，原始字节）、<code>Frequency</code> (u)、
<code>HwAddress</code> (s)、<code>Mode</code> (u)、<code>MaxBitrate</code> (u)、
<code>Strength</code>（y，0–100）、<code>LastSeen</code> (i)。

## Settings

路径：<code>/org/freedesktop/NetworkManager/Settings</code>

属性：<code>Hostname</code> (s)、<code>CanModify</code> (b)。

方法：

| 方法 | 签名 |
| --- | --- |
| ListConnections | () -> ao |
| GetConnection | (o) -> a{sa{sv}} |
| AddConnection | (a{sa{sv}}) -> o |
| AddConnectionUnsaved | (a{sa{sv}}) -> o |
| UpdateConnection | (o, a{sa{sv}}) -> b |
| DeleteConnection | (o) -> () |
| ReloadConnections | () -> b |
| SaveHostname | (s) -> () |

信号：<code>NewConnection(o)</code>、<code>ConnectionRemoved(o)</code>。

### Settings.Connection

路径：<code>/org/freedesktop/NetworkManager/Settings/&lt;counter&gt;</code>

- <code>GetSettings() -> a{sa{sv}}</code>（不含密钥）
- <code>GetSecrets(s service) -> a{sa{sv}}</code>（只返回安全子字典；
  允许 profile owner 或管理员）
- <code>Update(a{sa{sv}})</code>
- <code>UpdateUnsaved(a{sa{sv}})</code>
- <code>Delete()</code>

信号：<code>Updated()</code>、<code>Removed()</code>。

### Settings 字典示例

守护进程接受并输出标准嵌套字典。一个 WPA2 配置在 GVariant 文本形式下类似：

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

开放网络使用 <code>key-mgmt = 'none'</code>，或省略 security 设置。
SAE 使用 <code>key-mgmt = 'sae'</code>。

## Connection.Active

路径：<code>/org/freedesktop/NetworkManager/ActiveConnection/&lt;counter&gt;</code>

属性：<code>Connection</code> (o)、<code>SpecificObject</code> (o)、
<code>Id</code> (s)、<code>Uuid</code> (s)、<code>Type</code> (s)、
<code>Devices</code> (ao)、<code>State</code> (u)、<code>StateFlags</code> (u)、
<code>Default</code> (b)、<code>Default6</code> (b)、<code>Vpn</code> (b)、
<code>Master</code> (o)、<code>Ip4Config</code> (o)、<code>Dhcp4Config</code> (o)、
<code>Ip6Config</code> (o)、<code>Dhcp6Config</code> (o)。

## IP4Config

路径：<code>/org/freedesktop/NetworkManager/IP4Config/&lt;counter&gt;</code>

<code>AddressData</code>（aa{sv}，含 <code>address</code>/<code>prefix</code>）、
<code>Addresses</code> (aau)、<code>Gateway</code>（s，当前为空）、
<code>NameserverData</code> (aa{sv})、<code>Nameservers</code> (au)、
<code>Domains</code> (as)、<code>Routes</code> (aa{sv})、
<code>DnsOptions</code> (as)、<code>DnsPriority</code> (i)。

Wi-Fi 激活获得地址时、有线链路已有 IPv4 地址时，或观察到外部已建立的链路有地址时，
会创建该对象。对象读取实时设备状态，因此 DHCP 租约变化无需重建对象即可反映。

## 内部 DHCP 通知（非标准）

dhclient exit hook 调用一个内部对象，不对应 NetworkManager API：

    path:      /org/freedesktop/NetworkManager/Dhcp
    interface: org.freedesktop.NetworkManager.Dhcp
    method:    Notify(s ifname, s reason, a{ss} data)

<code>reason</code> 为 BOUND、RENEW、REBIND、EXPIRE、RELEASE、FAIL 或 TIMEOUT。
数据字典由 <code>nm-notify-dhcp</code> 从 dhclient 导出的
<code>new_ip_address</code>、<code>new_subnet_mask</code>、<code>new_routers</code>、
<code>new_domain_name_servers</code> 构建。方法要求管理员 caller；
D-Bus 策略不允许非特权用户调用。守护进程取第一个 router 作为 IP4Config Gateway，
第一个 DNS 地址作为 DNS。

ActiveConnection 状态在有地址前保持 ACTIVATING；NetworkManager 没有
“IP4_CONFIGURED” 这个 ActiveConnection 状态，该窗口对应 Device 状态
IP_CONFIG。

## 前端典型调用路径

plasma-nm/GNOME 的一次扫描/连接通常映射为：

1. <code>Manager.GetDevices</code> 或 <code>Devices</code> 属性。
2. 对每个设备做 <code>org.freedesktop.DBus.Properties.GetAll</code>，找
   <code>DeviceType == 2</code>。
3. <code>Device.Wireless.GetAllAccessPoints</code>；然后对每个 AP 读取
   <code>Ssid</code>、<code>Strength</code>、<code>Flags</code>、
   <code>WpaFlags</code>、<code>RsnFlags</code>。
4. <code>Device.Wireless.RequestScan</code>。
5. <code>Manager.AddAndActivateConnection(settings, device, "/")</code>。
6. 观察 <code>Connection.Active.State</code> 和 <code>Device.State</code>；
   激活后读取 <code>IP4Config</code>。
7. <code>Manager.DeactivateConnection(active)</code>。

<code>GetPermissions</code> 会通过总线守护进程解析 caller 的 Unix uid。
<code>wheel</code> 或 <code>network</code> 组成员对控制操作得到 <code>yes</code>，
其他本地用户得到 <code>no</code>（扫描仍为 <code>yes</code>）。
未实现的功能（WWAN、Wi-Fi 共享、按用户配置）即使是管理员也返回 <code>no</code>，
避免前端显示无法工作的控件。D-Bus 策略向管理员组授予完整访问权；
非特权用户可调用 owner 路径的激活/断开/GetSecrets，以及只读方法，
最终判定由 daemon 的 owner/admin 检查完成。

本 MVP 没有实现 <code>org.freedesktop.NetworkManager.SecretAgent</code> 或
<code>AgentManager</code>；请把 PSK 放在 settings 字典里（XFCE 插件就是这样做的）。
强制要求 agent 的前端需要后续补上该功能。保存的 PSK 从独立的
<code>keyring.json</code> 读取；<code>GetSecrets</code> 允许 profile owner 或管理员。

## 手动检查

    gdbus introspect --system \
      --dest org.freedesktop.NetworkManager \
      --object-path /org/freedesktop/NetworkManager

    gdbus call --system --dest org.freedesktop.NetworkManager \
      --object-path /org/freedesktop/NetworkManager \
      --method org.freedesktop.NetworkManager.GetDevices

    gdbus call --system --dest org.freedesktop.NetworkManager \
      --object-path /org/freedesktop/NetworkManager/Devices/3 \
      --method org.freedesktop.NetworkManager.Device.Wireless.RequestScan '{}'

（<code>busctl</code> 是 systemd 工具，FreeBSD 上不可用；<code>gdbus</code>、
<code>dbus-send</code> 和 Python dbus 可用。）
