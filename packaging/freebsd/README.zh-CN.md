# FreeBSD 打包

**语言：** [English](README.md) | [简体中文](README.zh-CN.md)

本目录包含 nm-freebsd 的部署文件和 Port skeleton。

## 目录结构

- <code>etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf</code>：系统总线策略。
- <code>etc/dbus-1/system-services/org.freedesktop.NetworkManager.service.in</code>：
  D-Bus activation。
- <code>etc/rc.d/nm_freebsd</code>：rc.d 脚本。
- <code>ports/net-mgmt/nm-freebsd</code>：Port skeleton。
  <code>cargo-crates.mk</code> 从 workspace 的 <code>Cargo.lock</code> 生成。

## 一键安装

在仓库根目录以 root 运行：

    sudo scripts/install.sh

它会检查依赖、构建 daemon（在主机有 GTK/XFCE 开发库时默认也构建插件）、安装
policy、activation service、rc.d 脚本和插件，启用并启动服务，应用 D-Bus 策略并运行冒烟测试。
可用 <code>--no-plugin</code>、<code>--no-start</code>、<code>--destdir</code>、
<code>--install-deps</code> 等选项；详见 <code>scripts/install.sh --help</code>。

安装器使用本目录中的文件，因此源码安装与 Port 安装保持一致。

## 安装 Port skeleton

把 <code>ports/net-mgmt/nm-freebsd</code> 复制到 ports tree 的
<code>net-mgmt/nm-freebsd</code>，把 <code>MAINTAINER</code> 改成真实地址
（<code>GH_ACCOUNT</code> 已指向 Nickhades），然后运行：

    make makesum
    make install clean

<code>CARGO_CRATES</code> 由 <code>tools/gen-port-crates.py</code> 生成。
<code>Cargo.lock</code> 变化后重新生成：

    python3 tools/gen-port-crates.py > packaging/freebsd/ports/net-mgmt/nm-freebsd/cargo-crates.mk

Port 的 XFCE 选项默认关闭；XFCE 系统上用 <code>make config</code> 或
<code>make -DWITH_XFCE=yes install clean</code> 启用。无头构建器不需要 GTK/XFCE。

## 手动安装（开发）

构建 daemon 和（可选）插件：

    cargo build --release -p nm-freebsd-daemon
    cargo build --release -p nm-xfce-plugin

以 root 安装：

    install -m 755 target/release/nm-freebsd-daemon /usr/local/bin/
    install -d /usr/local/etc/dbus-1/system.d /usr/local/share/dbus-1/system-services
    install -m 644 packaging/freebsd/etc/dbus-1/system.d/org.freedesktop.NetworkManager.conf \
      /usr/local/etc/dbus-1/system.d/
    sed 's|%%PREFIX%%|/usr/local|g' \
      packaging/freebsd/etc/dbus-1/system-services/org.freedesktop.NetworkManager.service.in \
      > /usr/local/share/dbus-1/system-services/org.freedesktop.NetworkManager.service
    sed 's|%%PREFIX%%|/usr/local|g' packaging/freebsd/etc/rc.d/nm_freebsd \
      > /usr/local/etc/rc.d/nm_freebsd
    chmod 755 /usr/local/etc/rc.d/nm_freebsd
    install -d /usr/local/lib/xfce4/panel/plugins /usr/local/share/xfce4/panel/plugins
    install -m 755 target/release/libnm_freebsd_xfce.so \
      /usr/local/lib/xfce4/panel/plugins/
    install -m 644 target/release/build/nm-xfce-plugin-*/out/nm_freebsd_xfce.desktop \
      /usr/local/share/xfce4/panel/plugins/

然后让总线读取新策略：

    service dbus restart
    sysrc nm_freebsd_enable=YES
    service nm_freebsd start

## DHCP hook

默认 DHCP 后端是 base <code>/sbin/dhclient</code>。一键安装器会安装
<code>/usr/local/libexec/nm-notify-dhcp</code> 和
<code>/usr/local/libexec/nm-freebsd-dhcp-hook</code>，然后向
<code>/etc/dhclient-exit-hooks</code> 追加一条带保护的 source 行：

    . /usr/local/libexec/nm-freebsd-dhcp-hook

Port 安装不会修改 <code>/etc</code>；请按 <code>pkg-message</code> 打印的行手工添加。
hook 把 BOUND/RENEW/EXPIRE 数据转发到内部 <code>Dhcp.Notify</code> 方法。
依赖租约驱动 Gateway 更新时不要移除 hook。

## 访问控制

D-Bus 策略只向 root 和 <code>wheel</code> 或 <code>network</code> 组成员授予完整控制权；
其他本地用户只有只读调用。如果桌面用户不在 <code>wheel</code>，把它加入
<code>network</code>：

    pw groupmod network -m <user>

然后注销并重新登录。在 polkit/SecretAgent 完成前，不要在共享、多用户或存在不可信本地用户的
机器上安装本 MVP。保存的 PSK 位于 <code>/var/db/nm-freebsd/keyring.json</code>（0600），
但没有静态加密。

## 说明

- 守护进程必须以 root 运行：wpa_supplicant 控制 socket 是 <code>root:wheel</code>，
  权限 0770。
- stock FreeBSD 的 wpa_supplicant 没有 D-Bus 接口，所以默认 auto 后端使用控制 socket。
- XFCE 插件是可选组件（Port 选项 XFCE，默认关闭）。
  在 XFCE 系统上启用；无头构建器不需要安装 GTK/XFCE。
