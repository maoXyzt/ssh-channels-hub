# Module Design / 模块设计文档

> English | [中文](#中文)

## English

### 1. Module map

```text
src/
|- main.rs        CLI dispatch, daemon, IPC, runtime orchestration
|- lib.rs         public module exports
|- cli.rs         clap command definitions
|- config.rs      TOML models and runtime channel resolution
|- ssh_config.rs  OpenSSH config parsing and default paths
|- host_check.rs  SSH alias support analysis
|- port_check.rs  listener and tunnel probes
|- service.rs     service lifecycle and route grouping
|- ssh.rs         SSH sessions, forwarding, verification, reconnection
|- web.rs         loopback Web status page
|- ui.rs          terminal rendering
`- error.rs       shared application errors
```

### 2. Responsibilities

#### `main.rs`

Initializes logging, dispatches commands, manages foreground and daemon modes,
stores runtime files, serves IPC requests, and coordinates `ServiceManager` and
the Web status server.

#### `lib.rs`

Exports the modules used by integration tests and downstream Rust code.

#### `cli.rs`

Defines global options and the `start`, `stop`, `restart`, `status`, `validate`,
`generate`, `hosts`, and `test` subcommands with `clap`.

#### `config.rs`

Deserializes `config.toml`, validates endpoints and directions, applies auth
overrides, resolves SSH aliases and jump chains, groups runtime values into
`ChannelConfig`, and generates configuration scaffolds.

Key types include `AppConfig`, `ConnectionConfig`, `ChannelConfig`, `Endpoint`,
`Direction`, `AuthConfig`, `JumpHopConfig`, `WebConfig`, and
`ReconnectionConfig`.

Authentication resolution is deterministic:

1. `[auth.<alias>].password` overrides key authentication.
2. Otherwise, the SSH config `IdentityFile` is used with an optional passphrase.
3. If neither exists, configuration fails before startup.

#### `ssh_config.rs`

Parses the supported OpenSSH directives, applies `Host *` defaults, resolves
`ProxyJump` and the supported `ProxyCommand` form, expands key paths, and
provides default SSH config, identity-file, and `known_hosts` paths.

#### `host_check.rs`

Analyzes SSH aliases for the `hosts` command and reports supported,
unsupported, and warning states without starting a connection.

#### `port_check.rs`

Checks whether listener addresses are available and probes configured tunnel
endpoints for the `test` command.

#### `service.rs`

Groups compatible channels by SSH route, owns their `SshManager` instances,
handles start and stop transitions, and aggregates `ChannelStatus` snapshots.
Channels share a session only when target host, SSH port, user, authentication,
jump chain, and remote-forward routing are compatible.

#### `ssh.rs`

Connects through jump chains, verifies all host keys, authenticates targets,
opens direct and forwarded TCP/IP channels, maps failures to retry decisions,
tracks per-channel health, and reconnects route groups with backoff and jitter.

`SshManager` is the main boundary. `ClientHandler` handles the target SSH session
and remote-forward callbacks; `JumpClientHandler` handles verified jump-host
sessions.

#### `web.rs`

Binds the loopback status server, renders service and channel state, builds
local endpoint links, escapes dynamic HTML, and displays host-key remediation
commands from channel errors.

#### `ui.rs`

Centralizes terminal styles, badges, tables, status output, and color disabling.

#### `error.rs`

Defines `AppError` and the shared `Result<T>` alias. Structured variants separate
configuration, connection, authentication, channel, I/O, and service failures.

### 3. Dependencies

```text
main
|- cli
|- config --> ssh_config
|- host_check --> ssh_config
|- port_check
|- service --> ssh --> config
|- web --> service
|- ui --> service
`- error (shared by runtime modules)
```

### 4. Design rules

- Keep parsing, orchestration, SSH transport, and presentation in separate
  modules.
- Resolve configuration before starting network tasks.
- Expose the smallest public API needed across module boundaries.
- Keep I/O asynchronous and avoid holding locks across `.await`.
- Use structured errors to classify permanent and retryable failures.

### 5. Extension points

- Channel types: extend configuration parameters and the forwarding branch in
  `ssh.rs`.
- Authentication: add an `AuthConfig` variant and terminal authentication path.
- Reconnection: extend `ReconnectionConfig` and the route-group retry loop.
- Metrics: consume `ServiceStatus` snapshots or add a dedicated exporter.

### 6. Testing

- Unit-test parsers, endpoint validation, route grouping, error mapping, HTML
  escaping, and remediation command formatting.
- Test async lifecycle and reconnection with deterministic time where possible.
- Keep live SSH-server tests as integration tests because they require external
  infrastructure.

## 中文

### 1. 模块概览

```text
src/
|- main.rs        CLI 分发、daemon、IPC 和运行时编排
|- lib.rs         公共模块导出
|- cli.rs         clap 命令定义
|- config.rs      TOML 模型和运行时 channel 解析
|- ssh_config.rs  OpenSSH config 解析和默认路径
|- host_check.rs  SSH alias 支持状态分析
|- port_check.rs  监听地址和隧道探测
|- service.rs     服务生命周期和路由分组
|- ssh.rs         SSH session、转发、校验和重连
|- web.rs         loopback Web 状态页
|- ui.rs          终端渲染
`- error.rs       共享应用错误
```

### 2. 模块职责

#### `main.rs`

初始化日志、分发命令、管理前台和 daemon 模式、维护运行时文件、处理 IPC，并协调
`ServiceManager` 与 Web 状态服务。

#### `lib.rs`

导出集成测试和下游 Rust 代码使用的模块。

#### `cli.rs`

使用 `clap` 定义全局选项以及 `start`、`stop`、`restart`、`status`、`validate`、
`generate`、`hosts` 和 `test` 子命令。

#### `config.rs`

反序列化 `config.toml`，校验 endpoint 和 direction，应用认证覆盖，解析 SSH alias
与跳板链，生成运行时 `ChannelConfig`，并生成配置脚手架。

主要类型包括 `AppConfig`、`ConnectionConfig`、`ChannelConfig`、`Endpoint`、
`Direction`、`AuthConfig`、`JumpHopConfig`、`WebConfig` 和
`ReconnectionConfig`。

认证解析顺序固定：

1. `[auth.<alias>].password` 覆盖密钥认证。
2. 否则使用 SSH config 的 `IdentityFile`，可附加 passphrase。
3. 两者都没有时，在启动前返回配置错误。

#### `ssh_config.rs`

解析受支持的 OpenSSH directive，应用 `Host *` 默认值，解析 `ProxyJump` 和兼容的
`ProxyCommand` 写法，展开密钥路径，并提供 SSH config、默认密钥和
`known_hosts` 路径。

#### `host_check.rs`

为 `hosts` 命令分析 SSH alias，在不建立连接的情况下报告支持、不支持和警告状态。

#### `port_check.rs`

检查监听地址是否可用，并为 `test` 命令探测已配置的隧道 endpoint。

#### `service.rs`

按 SSH 路由分组兼容的 channels，持有对应的 `SshManager`，管理启动和停止状态，
并汇总 `ChannelStatus` 快照。只有目标 host、SSH 端口、用户、认证、跳板链和远程
转发路由兼容时，channels 才会共享 session。

#### `ssh.rs`

连接跳板链、校验所有主机密钥、认证目标、打开 direct/forwarded TCP/IP channel、
将错误映射为重试决策、记录每条 channel 的健康状态，并使用 backoff 和 jitter
重连路由组。

`SshManager` 是主要边界。`ClientHandler` 处理目标 SSH session 和远程转发回调，
`JumpClientHandler` 处理已校验的跳板 session。

#### `web.rs`

绑定 loopback 状态服务、渲染服务和 channel 状态、生成本地 endpoint 链接、转义
动态 HTML，并展示 channel 错误中的主机密钥处置命令。

#### `ui.rs`

集中管理终端样式、badge、表格、状态输出和颜色开关。

#### `error.rs`

定义 `AppError` 和共享的 `Result<T>` 别名，通过结构化 variant 区分配置、连接、
认证、channel、I/O 和服务错误。

### 3. 模块依赖

```text
main
|- cli
|- config --> ssh_config
|- host_check --> ssh_config
|- port_check
|- service --> ssh --> config
|- web --> service
|- ui --> service
`- error（运行时模块共享）
```

### 4. 设计原则

- 将解析、编排、SSH 传输和展示拆分到不同模块。
- 在启动网络任务前完成配置解析。
- 模块间只暴露必要的最小公共 API。
- I/O 保持异步，不跨 `.await` 持锁。
- 使用结构化错误区分永久失败和可重试失败。

### 5. 扩展点

- Channel 类型：扩展配置参数和 `ssh.rs` 中的转发分支。
- 认证方式：增加 `AuthConfig` variant 和终点认证路径。
- 重连策略：扩展 `ReconnectionConfig` 和路由组重试循环。
- 指标：消费 `ServiceStatus` 快照或增加独立 exporter。

### 6. 测试策略

- 单元测试 parser、endpoint 校验、路由分组、错误映射、HTML 转义和处置命令格式。
- 对异步生命周期和重连尽量使用可控时间测试。
- 真实 SSH server 测试保留为集成测试，因为它依赖外部基础设施。
