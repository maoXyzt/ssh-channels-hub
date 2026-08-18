# SSH Channels Hub Documentation / 文档

> English | [中文](#中文)

## English

For an overview and quick start, see the [README](../README.md) or
[Chinese README](../README-zh.md).

### User documentation

- [Usage guide](./HowToUse.md): installation, configuration, and common tasks
- [Configuration reference](./configuration.md): fields, examples, and errors
- [Troubleshooting](./troubleshooting.md): connection, authentication, host-key,
  and port problems
- [Connection testing](./testing.md): verify configured channels

### Design documentation

- [Architecture](./architecture.md): components and concurrency model
- [Module design](./modules.md): module responsibilities and dependencies
- [Workflows](./workflow.md): startup, connection, reconnection, and shutdown

### Maintainer documentation

- [Release guide](./how_to_release.md): publishing and withdrawal procedures
- [Development guidelines](../AGENTS.md): code conventions and contribution rules

Recommended order:

- Users: README -> Usage guide -> Configuration reference
- Developers: Architecture -> Module design -> Workflows
- Maintainers: Development guidelines -> Release guide

See also [config.example.toml](../config.example.toml) and
[LICENSE](../LICENSE).

## 中文

项目概览和快速开始见 [README](../README.md) / [中文 README](../README-zh.md)。

### 用户文档

- [使用教程](./HowToUse.md)：安装、配置和常用操作
- [配置参考](./configuration.md)：完整字段、示例和配置错误
- [故障排查](./troubleshooting.md)：连接、认证、主机密钥和端口问题
- [连接测试](./testing.md)：验证配置的 channel

### 设计文档

- [架构设计](./architecture.md)：整体架构和并发模型
- [模块设计](./modules.md)：模块职责和依赖关系
- [工作流程](./workflow.md)：启动、连接、重连和关闭流程

### 维护文档

- [发版手册](./how_to_release.md)：版本发布和撤回流程
- [开发规范](../AGENTS.md)：代码约定和贡献要求

推荐阅读顺序：

- 新用户：README -> 使用教程 -> 配置参考
- 开发者：架构设计 -> 模块设计 -> 工作流程
- 维护者：开发规范 -> 发版手册

另见 [config.example.toml](../config.example.toml) 和 [LICENSE](../LICENSE)。
