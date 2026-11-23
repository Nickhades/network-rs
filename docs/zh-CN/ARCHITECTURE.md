# 架构

**语言：** [English](../ARCHITECTURE.md) | [简体中文](ARCHITECTURE.md)

## 分层

1. **桌面探测与分发**（<code>crates/nm-common/src/desktop.rs</code>、
   <code>crates/nm-freebsd-daemon/src/daemon.rs</code>）只读取一次 XDG 桌面变量并记录预期前端。
   它绝不启动 UI。KDE 和 GNOME 使用原生前端；XFCE 使用 Port 安装的插件；未知桌面只提供 D-Bus。
2. **D-Bus 服务**（<code>crates/nm-freebsd-daemon/src/dbus/</code>）是门面。zbus 拥有
   <code>org.freedesktop.NetworkManager</code>，导出 Manager、Device、AccessPoint、Settings、
   ActiveConnection、IP4Config 对象，并发出 NetworkManager 形状的信号。
3. **核心**（<code>crates/nm-freebsd-daemon/src/core.rs</code>）持有规范状态，负责对象生命周期
   （动态 AP/连接注册与注销）、激活状态机以及通知发送。
4. **后端**（<code>crates/nm-freebsd-daemon/src/backend/</code>）把命令翻译成 wpa_supplicant 操作。
   <code>netlink.rs</code> 和 <code>system.rs</code> 提供 FreeBSD 的链路/事件/地址事实。
   不解析 <code>ifconfig</code> 命令输出。

桌面插件是独立的 cdylib，也是纯 D-Bus 客户端；它只共享 <code>nm-common</code> 中的常量、桌面探测和 SSID 辅助函数。

## 状态模型

<code>CoreState</code> 由 <code>tokio::sync::RwLock</code> 保护，包含：

- <code>devices: HashMap&lt;ifname, DeviceEntry&gt;</code>
- <code>aps: HashMap&lt;"ifname/bssid", ApEntry&gt;</code>
- <code>connections: HashMap&lt;OwnedObjectPath, ConnectionProfile&gt;</code>
- <code>active: HashMap&lt;OwnedObjectPath, ActiveEntry&gt;</code>
- Manager 标志（状态、连通性、网络/Wi‑Fi 启用、primary）
- AP、连接、活动连接、IP4Config 对象路径的单调递增计数器

对象路径在对象生命周期内保持稳定：

    /org/freedesktop/NetworkManager
    /org/freedesktop/NetworkManager/Devices/<ifindex>
    /org/freedesktop/NetworkManager/AccessPoints/<counter>
    /org/freedesktop/NetworkManager/Settings
    /org/freedesktop/NetworkManager/Settings/<counter>
    /org/freedesktop/NetworkManager/ActiveConnection/<counter>
    /org/freedesktop/NetworkManager/IP4Config/<counter>

设备路径使用接口索引，因此相同索引的设备重新出现时复用路径。AP 以“接口 + BSSID”为键；
不同 BSSID 使用相同 SSID 时仍是不同 AP，这一点与 NetworkManager 一致。

## 避免死锁

第一版用 <code>ObjectServer::interface(..).get_mut()</code> 发送属性变化。在 D-Bus 方法内部调用时这会死锁：
dispatcher 已经持有该接口的读锁，而处理函数又等待同一个对象的写锁。

因此所有通知都直接通过 <code>SignalEmitter::new(&amp;connection, path)</code> 发送：

- 从状态快照手工构造 <code>org.freedesktop.DBus.Properties.PropertiesChanged</code>；
- Manager <code>StateChanged</code>、Device <code>StateChanged</code>、AP added/removed、
  settings new/removed、活动连接通知使用 <code>SignalEmitter::emit</code>。

这也让对象服务器远离热路径。

<code>State</code> 属性使用
<code>#[zbus(property(emits_changed_signal = "false"))]</code>，
避免宏生成与 <code>StateChanged</code> 信号冲突的 <code>state_changed</code> 辅助方法；
该属性由手工 PropertiesChanged 字典覆盖。

## 后端抽象

<code>backend::Backend</code> 是一个异步 trait：

    async fn scan(ifname) -> Result<(), BackendError>;
    async fn scan_results(ifname) -> Result<Vec<ScanAp>, BackendError>;
    async fn status(ifname) -> Result<WifiStatus, BackendError>;
    async fn connect(&ConnectRequest) -> Result<(), BackendError>;
    async fn disconnect(ifname) -> Result<(), BackendError>;
    fn spawn_events(Arc<Self>, Sender<BackendEvent>);

两个实现：

- <code>wpa_ctrl</code>：datagram Unix socket，是 FreeBSD 基础系统唯一可用的传输。
  命令使用短生命周期且已绑定路径的客户端 socket；事件在 <code>ATTACH</code> 后使用一个长连接。
  详见 [FREEBSD-NOTES.md](FREEBSD-NOTES.md)。
- <code>wpa_dbus</code>：<code>fi.w1.wpa_supplicant1</code> 代理，用于定制构建。它调用
  <code>Interface.Scan</code>，监听 <code>ScanDone</code>，然后读取 <code>BSSs</code> 属性；
  状态每 2 秒轮询一次，10 秒一次的 BSS 轮询作为兜底。Auto 模式只有在总线名被拥有时才选择它。

<code>Auto</code> 会探测 D-Bus 名称，否则选择控制 socket。显式 <code>--backend</code>
在 auto 模式之外不会被静默降级。debug 级别会记录“D-Bus 名称不可用、auto 回退控制 socket”，
避免 ports 构建启用 wpa_supplicant D-Bus 选项后行为变化却没有提示。

## 安全与授权

访问控制有三层：

1. 安装的 D-Bus 策略向 root 和 <code>wheel</code>/<code>network</code> 组成员授予完整
   send 权限；其他本地用户只允许明确的只读方法（<code>Properties.Get/GetAll</code>、
   Manager 查询、AP 发现/扫描、不含密钥的 settings 元数据）。
2. <code>auth.rs</code> 通过 <code>org.freedesktop.DBus.GetConnectionUnixUser</code>
   解析 caller 的 Unix uid，检查 root 或上述组成员资格。守护进程实现的每个修改类方法都调用
   <code>require_admin</code>；<code>GetPermissions</code> 把同样的判断返回给前端。
   <code>GetSecrets</code> 只允许管理员。
3. 保存的 PSK 位于 <code>/var/db/nm-freebsd/keyring.json</code>（0600），与 0600 的
   profile JSON 分离。旧 profile 中的明文 <code>psk</code> 在加载时迁移。
   keyring 没有静态加密，root 可以读取；这是隔离/最小暴露措施，不是密钥保险库。

目前仍没有 polkit 或 SecretAgent。不属于管理员组的桌面用户会得到只读界面；
README 警告说明了如何通过 <code>network</code> 组授权。

## 激活

<code>AddAndActivateConnection</code> 和 <code>ActivateConnection</code> 走同一核心路径：

1. 解析或创建配置。连接路径为 <code>/</code> 且指定 AP 时，根据 AP 的 SSID 和安全类型建立未保存配置。
2. 解析设备。<code>/</code> 表示第一个匹配该配置的 Wi-Fi 设备。
3. 停用该设备上已有的活动连接。
4. 插入状态为 <code>ACTIVATING</code> 的 <code>ActiveEntry</code>，设备置为 <code>PREPARE</code>。
5. 注册 <code>Connection.Active</code>，然后启动激活任务。
6. 任务设置 <code>CONFIG</code>，调用 <code>Backend::connect</code>，每 500 ms 轮询一次
   <code>Backend::status</code>，最长 20 秒。
7. 收到 <code>COMPLETED</code>：设置 <code>ACTIVATED</code>；有地址时发布 IP4Config；
   活动连接置为 <code>ACTIVATED</code> 和 default；重算 Manager 状态；发送 device/active/manager 信号。
8. 失败/超时/断开：用 NetworkManager 状态原因把设备标为 <code>FAILED</code>，发送信号，
   移除活动对象并回到 <code>DISCONNECTED</code>。

<code>DeactivateConnection</code> 设置 <code>DEACTIVATING</code>，调用
<code>Backend::disconnect</code>，移除 IP4Config 和活动对象，清理设备状态并重算 Manager。
激活任务在每次更新前检查活动路径是否还存在，因此并发停用会让它自然取消。

## 事件

两个事件泵驱动核心：

- **wpa_supplicant 事件：** <code>CTRL-EVENT-SCAN-RESULTS</code> 触发 AP 刷新；
  connected/disconnected/auth 事件触发 STATUS 查询和状态更新。
- **netlink：** link add/delete/change 更新设备存在性与状态；地址变化刷新 IPv4 信息。
  netlink 监听无法创建时，使用 5 秒一次的 getifaddrs 轮询。

AP 刷新由互斥锁串行化。新 AP 先注册为对象，再发送 <code>AccessPointAdded</code>；
移除的 AP 在发送 <code>AccessPointRemoved</code> 后注销。空扫描保留旧列表以避免 UI 闪烁。

## 配置

连接配置是 JSON 文件，而不是序列化的 zvariant map。只保存后端实际能应用的字段；
<code>GetSettings</code> 合成标准 <code>a{sa{sv}}</code> 字典，<code>GetSecrets</code>
返回安全子字典。这样磁盘格式可检查，也避免持久化任意不可信 variant 数据。

## 桌面插件生命周期

XFCE 插件由 xfce4-panel 加载到面板进程内。它创建一个 GTK 按钮和一个使用 current-thread Tokio
runtime 的 worker 线程。worker 负责所有 zbus 工作；GTK 主线程只通过 150 ms 定时源消费
<code>UiEvent</code>。

每次请求都会新建系统总线连接。守护进程重启后，下一次 5 秒刷新或菜单操作会自动重连；
插件没有会失效的粘性连接。移除插件时会释放定时器源和 worker 的关闭请求。
