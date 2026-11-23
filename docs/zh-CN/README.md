# 文档

**语言：** [English](../README.md) | [简体中文](README.md)

英文文件是规范文档，本目录是同一内容的完整中文镜像。行为变化时两份都要更新。

| 文档 | English | 简体中文 |
| --- | --- | --- |
| 项目说明 | [README.md](../../README.md) | [README.zh-CN.md](../../README.zh-CN.md) |
| 架构 | [ARCHITECTURE.md](../ARCHITECTURE.md) | [ARCHITECTURE.md](ARCHITECTURE.md) |
| D-Bus 接口 | [DBUS.md](../DBUS.md) | [DBUS.md](DBUS.md) |
| Wi-Fi 扫描链路 | [SCAN-FLOW.md](../SCAN-FLOW.md) | [SCAN-FLOW.md](SCAN-FLOW.md) |
| FreeBSD 实现备注 | [FREEBSD-NOTES.md](../FREEBSD-NOTES.md) | [FREEBSD-NOTES.md](FREEBSD-NOTES.md) |
| 测试 | [TESTING.md](../TESTING.md) | [TESTING.md](TESTING.md) |
| 代码审核 | [CODE-REVIEW.md](../CODE-REVIEW.md) | [CODE-REVIEW.md](CODE-REVIEW.md) |
| FreeBSD 打包 | [packaging/freebsd/README.md](../../packaging/freebsd/README.md) | [README.zh-CN.md](../../packaging/freebsd/README.zh-CN.md) |

## 维护约定

- 英文文档是规范版本；中文文档是完整翻译，不另立一套行为描述。
- 修改接口、默认值、限制或测试命令时，同一次提交中同时更新英文和中文。
- 代码、类型名、D-Bus 方法/属性/信号名和命令保持英文，避免翻译后无法搜索。
