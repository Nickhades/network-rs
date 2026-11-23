# 代码审核

**语言：** [English](../CODE-REVIEW.md) | [简体中文](CODE-REVIEW.md)

范围：本仓库的 Rust workspace、shell 安装/测试工具和 FreeBSD 打包。这是一次工作审核，不是正式安全审计。

## 审核结论

本轮覆盖守护进程/插件、安装与测试脚本、FreeBSD 打包。重点修复了
wpa_supplicant 控制接口 datagram 类型、D-Bus 方法内自死锁、非 ASCII SSID 的 wpa 转义、
配置文件 UUID 路径穿越、插件读取 <code>/dev/urandom</code> 阻塞、激活/断开竞态、
首次激活 IP4Config 缺失和有线接口状态等问题。Phase 2 已补上
dhclient/dhcpcd DHCP、exit-hook 租约事件、Gateway/DNS、有线激活和
CONNECTED_SITE 超时语义。剩余主要缺口是 SecretAgent/polkit、IPv6、dhcpcd 专用
lease hook，以及尚未在真实 plasma-nm/GNOME 面板上验证。自动测试、私有总线冒烟、
真实 wpa 只读扫描和隔离 XFCE 面板渲染均已通过。

## 审核时验证

- <code>cargo fmt --all --check</code>
- <code>cargo clippy --workspace --all-targets -- -D warnings</code>
- <code>cargo test --workspace</code>：16 个 common + 33 个 daemon + 1 个 plugin
  + 2 个端到端集成测试
- daemon 和 XFCE 插件 release 构建
- 插件 <code>dlopen</code> + <code>xfce_panel_module_init</code> 符号检查
- 使用 <code>--destdir</code> 的 staged 一键安装/卸载
- 对私有系统总线、假 wpa_supplicant 和真实 wpa_supplicant 的只读冒烟
- 隔离 Xvfb/XFCE 面板渲染测试，包含面板和菜单截图
  （<code>artifacts/desktop-test/</code>）

## 已修复问题

| 严重度 | 区域 | 问题 | 修复 |
| --- | --- | --- | --- |
| 严重 | wpa 后端 | FreeBSD wpa_supplicant 控制 socket 是 <code>SOCK_DGRAM</code>；原 stream 客户端在真实硬件返回 <code>EPROTOTYPE</code>。 | datagram 客户端，私有客户端 bind，命令不带换行，每个 datagram 即一个回复；测试假服务器同步修改。 |
| 高 | D-Bus 通知 | 在 D-Bus 方法内部用 <code>ObjectServer::interface(..).get_mut()</code> 发送属性变化，会与 dispatcher 的读锁死锁。 | 所有通知直接用 <code>SignalEmitter</code> 和显式 <code>PropertiesChanged</code> 字典。 |
| 高 | XFCE 插件 | <code>std::fs::read("/dev/urandom")</code> 会读到 EOF，导致 worker 永久阻塞。 | 打开文件后 <code>read_exact</code> 16 字节，失败时用时间兜底。 |
| 高 | 配置 | 客户端提供的 UUID 变成文件名，可路径穿越（<code>../../...</code>）。 | 输入时解析/重新生成 UUID，加载和保存时校验；测试覆盖。 |
| 高 | 密钥 | Wi-Fi PSK 与普通配置元数据一起明文存放在 profile JSON。 | 移到独立的 0600 <code>keyring.json</code>，迁移旧 <code>psk</code> 字段，profile JSON 不再包含密钥。 |
| 高 | 授权 | 策略允许所有本地用户执行完整方法调用，<code>GetPermissions</code> 永远返回 yes。 | 完整权限限制到 root/<code>wheel</code>/<code>network</code>；守护进程对修改类方法检查 caller uid；<code>GetPermissions</code> 解析 uid，其他用户只读。 |
| 中 | 配置 | 同一 UUID 添加第二个配置会创建两个路径并覆盖同一个 JSON。 | 复用已有路径并发送 <code>Updated</code>。 |
| 中 | 激活 | 停用操作可能落在激活任务最后一次检查和状态写入之间，留下 ACTIVATED 设备。 | 完成/失败路径在写锁内重新检查活动对象存在且不是 DEACTIVATED。 |
| 中 | IP4Config | 激活时在把新地址写入设备之前创建 IP4Config，首次连接永远创建不出对象。 | 先发布地址再调用 <code>ensure_ip4_config</code>；断开时清理旧对象。 |
| 中 | 有线链路 | 已有 IPv4 的以太网设备被报告为 unavailable，且没有 IP4Config。 | 链路/地址更新设置 ACTIVATED 并创建/移除 IP4Config。 |
| 中 | Manager 状态 | 有线设备出现/激活时没有重算 Manager 状态。 | 在 upsert 和地址变化时重算。 |
| 中 | XFCE 插件 | 点击闭包与按钮形成 <code>Rc</code> 环；定时器在插件移除后仍存活。 | 闭包使用 Weak；<code>free_data</code> 移除 <code>SourceId</code>。 |
| 低 | D-Bus | <code>Device.Wireless.RequestScan</code> 类型误为 <code>a{sa{sv}}</code>，应为 <code>a{sv}</code>。 | 修正签名，集成测试覆盖。 |
| 低 | D-Bus | Manager/Device 的 <code>State</code> 属性变化辅助方法与 <code>StateChanged</code> 冲突。 | 使用 <code>emits_changed_signal = "false"</code> + 显式信号声明和手工发送。 |
| 低 | AP | 已有 AP 的信号强度变化时没有 <code>PropertiesChanged</code>。 | 每次扫描后发送每个 AP 的变化。 |
| 低 | wpa 后端 | <code>SAVE_CONFIG</code> 可能悄悄改写 <code>/etc/wpa_supplicant.conf</code>。 | 移除；配置保存在守护进程状态目录。 |
| 低 | wpa D-Bus | 后端只轮询 BSS，没有订阅真实 <code>ScanDone</code> 信号。 | 监听 <code>Interface.ScanDone</code> 后刷新 BSS；mock 集成测试证明链路。 |
| 低 | 后端选择 | auto 回退到控制 socket 时没有说明原因，debug 级别也看不到。 | debug 日志明确记录 <code>fi.w1.wpa_supplicant1</code> 不可用、auto 使用控制 socket。 |
| 高 | IP 配置 | 关联完成时即使没有 IPv4 地址，设备也会被标记为 ACTIVATED，活动连接也标记为 ACTIVATED。 | DHCP 期间设备进入 IP_CONFIG，活动连接保持 ACTIVATING；有地址后才完成；超时降级 CONNECTED_SITE。 |
| 高 | DHCP 集成 | 有线 ActivateConnection 返回 NotSupported，且没有租约事件路径。 | 有线使用同一 DHCP 分支；dhclient exit hook 调用内部 Dhcp.Notify；更新 IP4Config Gateway/DNS。 |
| 中 | FreeBSD 路径 | 最初假设 <code>/var/run/dhclient.*.pid</code>；FreeBSD 实际使用 <code>/var/run/dhclient/dhclient.&lt;ifname&gt;.pid</code>。 | 使用真实路径，复用存活的系统 dhclient，避免重复 fork。 |
| 低 | 脚本 | 冒烟测试 GVariant 数字解析把类型宽度也算进去（<code>&lt;uint32 50&gt;</code> 变成 3250），选项循环会跳过值。 | 取最后一个字段、去前导零，改用 <code>while</code> 参数循环。 |
| 低 | 脚本 | 隔离桌面测试把 D-Bus socket 放在含非 ASCII 字节的路径下，D-Bus 拒绝。 | 运行时状态放 <code>/tmp</code>，截图/日志复制回仓库。 |
| 低 | 打包 | Port Makefile 在 <code>bsd.port.options.mk</code> 之前使用 <code>PORT_OPTIONS</code>，并假设 target 目录。 | 加入 <code>bsd.port.options.mk</code> 和显式 <code>CARGO_BUILD_TARGET_DIR</code>。 |
| 低 | Wi-Fi | wpa_supplicant 把非 ASCII SSID 打印为字面量 <code>\xNN</code>；AP 的 <code>Ssid</code> 变成反斜杠 ASCII。 | <code>nm-common</code> 的 <code>unescape_wpa</code> 用于扫描/状态解析，并有测试；真实扫描确认恢复原始 UTF-8 字节。 |

## 接受的风险 / 待办

- DHCP 已实现：默认 base <code>dhclient</code>，可选 <code>dhcpcd</code>。
  守护进程复用 rc.conf 已启动的 dhclient，否则启动受管前台客户端。
  超时后设备保持 IP_CONFIG，Manager 降级到 CONNECTED_SITE，而不是谎报地址。
- <code>dhcpcd</code> 路径是实验性的：本 MVP 没有 dhcpcd 专用租约 hook，
  地址更新依赖普通地址轮询，Gateway 可能为空。没有执行真实 root 所有的
  dhclient/hook 测试；staged 安装和模拟 <code>Dhcp.Notify</code> 路径已测试。
- 不支持 IPv6、DHCPv6、DHCP option 对象、SecretAgent、polkit、VPN、802.1X、WEP、
  隐藏网络 UI、已保存配置自动激活。
- 可选 wpa_supplicant D-Bus 后端可编译，但未在真实
  <code>fi.w1.wpa_supplicant1</code> 服务上运行（stock FreeBSD 没有）；mock 测试覆盖协议链路。
- 尚未测试真实 plasma-nm 和 GNOME 面板；XFCE 已在隔离的真实面板中测试。
- 完整 D-Bus 控制仍限制到 <code>wheel</code>/<code>network</code>；profile owner
  可以激活/断开自己的 profile、编辑/删除自己的 profile 并读取自己的密钥。
  Enable/Sleep 和整机设置仍要求管理员组。
  仍然没有 polkit/SecretAgent。keyring 是独立的 0600 文件，但没有静态加密，
  root（以及能读取 root 文件的人）可以恢复 PSK。
- 不属于 <code>wheel</code>/<code>network</code> 的桌面用户可以管理自己的 profile，
  但编辑系统设置前仍需要加入其中一个组。
- 本机 FreeBSD GNOME Control Center 没有 network 面板，也没有链接 <code>libnm</code>；
  <code>networkmanager-shim</code> 是编译空壳。D-Bus 服务无法让编译时未启用 NM 的前端
  出现面板。Phase 4 需要功能性 libnm/NetworkManagerQt 和重新构建的桌面包。
- 如果 <code>activate</code> 第一次 <code>ObjectServer::at</code> 注册期间发生停用，
  可能留下孤立的 active 对象；窗口很窄（回调返回前立即停用），重启守护进程会清理。
  每设备激活互斥锁可以彻底关闭该窗口。
- XFCE 插件每次连接生成新的 profile UUID，重复连接同一 SSID 会创建重复配置；
  守护进程能保持一致，但做配置查找会更优雅。
- 隔离桌面测试点击固定面板坐标，并要求真实 <code>wlan0</code> 用于设备枚举；
  它是测试工具，不是通用 UI 自动化框架。
- Port skeleton 的 <code>MAINTAINER</code> 是占位值，且没有 <code>distinfo</code>；
  在真实 ports tree 中运行 <code>make makesum</code>。<code>GH_ACCOUNT</code> 已指向
  <code>Nickhades/network-rs</code>。
- 当前环境没有 root/sudo 密码，因此未执行完整 root 安装；同一路径已用
  <code>--destdir</code> 验证，服务/冒烟/桌面路径在私有总线上验证。

## FreeBSD 底层审核

- netlink 在读取前校验消息和 TLV 长度；每次 FreeBSD 测试运行都会执行真实
  RTM_GETLINK dump 测试。
- 显式处理 <code>AF_NETLINK = 38</code>、FreeBSD 长度前缀
  <code>sockaddr_nl</code> 和 host-endian 头；不使用 Linux-only netlink crate。
- getifaddrs 内存用 <code>freeifaddrs</code> 释放；接口地址在列表释放前复制。
- wpa datagram 客户端绑定唯一路径，权限 0600，drop 时 unlink；命令不经过 shell。
- 不解析 <code>ifconfig</code> 输出。接口状态/地址来自 netlink/getifaddrs；
  Wi-Fi 控制来自 wpa_supplicant。
