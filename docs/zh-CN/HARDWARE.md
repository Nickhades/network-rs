# 硬件与频段支持

本项目是 NetworkManager 兼容的控制面。它只能管理 FreeBSD 内核驱动已经暴露的
接口和 AP；它不是无线驱动，不能凭空增加信道、频段或 firmware 支持。

## 为什么某个 SSID 或频段会消失

802.11 的 SSID 是 0–32 字节的字节串，不是 Unicode 字符串。中文名是 UTF-8
字节；`wpa_cli` 显示为 `\xNN`，`ifconfig` 可能显示短十六进制形式。
daemon 保留原始字节，CLI 按 UTF-8 显示。十六进制显示不等于网络丢失。

以下情况不会出现在 `nm-freebsd-cli wifi list`：

- 驱动没有暴露该信道/频段；
- AP 使用的信道不在当前 regulatory/driver 信道表中；
- AP 是隐藏网络、超出范围或没有 beacon；
- 网卡/firmware 不支持该模式（例如 5GHz VHT）。

`nm-freebsd-cli doctor` 会同时打印：

- `ifconfig <iface> list chan` 报告的支持信道；
- 当前扫描实际可见的频段。

“网卡支持”和“现在能扫到”是两件不同的事。

## 支持矩阵（指导，不是保证）

支持情况取决于 FreeBSD 版本、firmware、国家码和具体芯片。始终用下面命令确认：

```sh
ifconfig <iface> list chan
nm-freebsd-cli doctor
```

| 驱动/系列 | 常见频段 | 本项目状态 |
| --- | --- | --- |
| `iwlwifi`（Intel） | 2.4/5GHz；支持的芯片可有 6GHz | 在 FreeBSD 驱动/firmware 支持该芯片时推荐 |
| `ath`、`ath10k`、`ath11k` | 2.4/5GHz；6GHz 视驱动 | FreeBSD 驱动暴露接口时可用 |
| `urtwn`、`run`、`rtwn`（USB） | 多为 2.4GHz，部分 5GHz | 需按 USB ID 和驱动确认 |
| `rtw88`、`rtw89` | 2.4GHz；有限 5GHz | 实验性；存在稳定性问题，包括 kernel panic |
| 有线（`em`、`igb`、`re`、USB Ethernet） | 仅有线 | 由 daemon 完整管理 |

这不是兼容性承诺。芯片可能被 FreeBSD 支持，但仍受 firmware、国家码或天线限制。

### 本测试环境的 rtw89

当前 RTL8852BE 测试机只暴露 2.4GHz 和 5GHz 的 149/153/157/161，并且：

```text
compat.linuxkpi.rtw89_support_ht  = 0
compat.linuxkpi.rtw89_support_vht = 0
compat.linuxkpi.rtw89_support_eht = 0
```

连续关联时出现过 kernel page fault。不要把 rtw89 宣传为可靠的下游 5GHz 方案。

## 分发建议

- 提供支持矩阵，不要承诺“所有无线网卡都支持”。
- 运行时探测接口，不要只硬编码 `wlan0`。
- 明确报告不支持的频段，而不是静默隐藏网络。
- Wi-Fi 受限时，保留 Ethernet/USB 共享网络可用。
- 隐藏 SSID 使用显式 profile（`802-11-wireless.hidden=true`）和
  <code>nm-freebsd-cli wifi connect --hidden</code>；普通扫描无法列出不广播的网络。
- 在实际用户使用的 FreeBSD 版本和 firmware 上测试。

## 项目能做与不能做

能做：

- 枚举 `wlan*` 和 Ethernet 接口；
- 扫描、连接并报告驱动返回的 AP；
- 按 SSID 显示频段和 BSSID 数量；
- 诊断支持信道与可见 BSS 的差异；
- 管理已存在接口的 DHCP 和地址。

不能做：

- 让驱动扫描它没有暴露的信道；
- 把只支持 2.4GHz 的网卡变成 5GHz 网卡；
- 从普通扫描中列出隐藏 SSID；
- 保证某个芯片在 FreeBSD 下稳定。
