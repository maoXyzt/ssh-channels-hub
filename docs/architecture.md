# Architecture / 架构设计

> English | [中文](#中文)

## English

### 1. Scope

SSH Channels Hub is a Tokio-based CLI service that maintains configured SSH
port forwards. Compatible channels share an SSH session; independent SSH routes
run separately and reconnect without stopping the whole service.

### 2. System context

```text
CLI commands
    |
    v
main runtime ---------> IPC status/stop
    |                   Web status page
    v
configuration pipeline (config.toml + SSH config + preflight checks)
    |
    v
ServiceManager
    |
    +-- SshManager (route group 1) -- russh session -- channel tasks
    +-- SshManager (route group N) -- russh session -- channel tasks
```

The main runtime owns process-level concerns: command dispatch, foreground and
daemon modes, runtime files, IPC, Web status, logging, and shutdown.

### 3. Core components

#### Interfaces

- `cli.rs` defines commands and global options with `clap`.
- `ui.rs` renders terminal output and per-channel health.
- `web.rs` serves the loopback status page and remediation commands.
- `main.rs` implements local IPC for `status`, `stop`, and `restart`.

#### Configuration pipeline

- `config.rs` parses TOML and builds runtime channel configuration.
- `ssh_config.rs` resolves OpenSSH aliases, defaults, keys, and jump chains.
- `host_check.rs` evaluates aliases for the `hosts` command.
- `port_check.rs` checks listener availability and probes active tunnels.

Configuration is fully resolved before network tasks start. This keeps invalid
aliases, endpoints, authentication, and jump-host setup out of the runtime
connection loop.

#### Service lifecycle

`service.rs` groups channels by SSH route, owns their `SshManager` instances,
and aggregates `ChannelStatus` snapshots. Target, port, user, authentication,
and `ProxyJump` chain must match before channels can share a session. Conflicting
nonzero remote-forward ports are kept in separate sessions.

#### SSH transport

`ssh.rs` connects through jump hosts, verifies host keys, authenticates the
target, registers local and remote forwards, tracks health, and reconnects route
groups. `russh` supplies the SSH protocol implementation.

See [Module design](./modules.md) for file-level responsibilities.

### 4. Data flow

#### Startup

```text
Load config.toml
  -> resolve SSH aliases and authentication
  -> validate endpoints and resolve jump-host configuration
  -> group compatible channels
  -> connect and authenticate each route
  -> start local listeners or register remote forwards
  -> publish health through IPC and Web status
```

#### Forwarding

- `local->remote`: a local `TcpListener` accepts connections and opens one
  `direct-tcpip` SSH channel per connection.
- `remote->local`: the client registers `tcpip-forward`; each
  `forwarded-tcpip` callback connects to the configured local endpoint.

Both directions copy data asynchronously in both directions.

### 5. Concurrency and state

- Each SSH route group runs in its own Tokio task.
- Compatible channels share one SSH session; accepted TCP connections use child
  tasks.
- A global permit serializes SSH handshakes to avoid reconnection storms.
- `CancellationToken` coordinates process, IPC, Web, manager, and listener
  shutdown.
- Mutexes protect short service and health updates and are not held across
  network `.await`s.

Channel health moves through `Stopped`, `Connecting`, `Connected`,
`Reconnecting`, and `Failed`. `ServiceManager` exposes snapshots rather than
transport internals.

### 6. Failure and reconnection

- Configuration errors fail before startup.
- Retryable transport or session errors rebuild the affected route group using
  the configured backoff plus jitter.
- Authentication and host-key failures are permanent until configuration or
  trust data changes.
- A permanent channel-specific failure disables that channel while compatible
  channels continue when possible.
- A retryable channel failure may rebuild the shared session and all channels in
  that route group.

After a finite retry cycle is exhausted, recovery continues with a second
jittered exponential backoff capped at 60 seconds. A successful session resets
both retry levels.

### 7. Security boundaries

- Targets and jump hosts are strictly verified against `~/.ssh/known_hosts`.
- Unknown and changed keys are rejected; CLI and Web errors provide remediation
  commands but never trust keys automatically.
- Jump hosts support public-key authentication only and require unencrypted
  keys.
- Passwords and passphrases in `config.toml` are plaintext; key authentication
  is preferred and file permissions should restrict access.
- IPC and the Web status page listen only on loopback.
- Available SSH algorithms are those enabled by the current `russh`
  configuration.

### 8. Observability

Structured `tracing` events include route, channel, host, retry, and error
context. The CLI and Web page consume the same per-channel health snapshots, so
their status semantics remain aligned.

For detailed execution sequences, see [Workflows](./workflow.md).

## 中文

### 1. 范围

SSH Channels Hub 是一个基于 Tokio 的 CLI 服务，用于维护配置的 SSH 端口转发。
兼容的 channels 共享 SSH session；不同 SSH 路由独立运行和重连，不会停止整个
服务。

### 2. 系统上下文

```text
CLI 命令
    |
    v
main 运行时 ---------> IPC 状态/停止
    |                  Web 状态页
    v
配置流水线（config.toml + SSH config + 前置检查）
    |
    v
ServiceManager
    |
    +-- SshManager（路由组 1）-- russh session -- channel 任务
    +-- SshManager（路由组 N）-- russh session -- channel 任务
```

main 运行时负责进程级能力：命令分发、前台和 daemon 模式、运行时文件、IPC、
Web 状态、日志和关闭流程。

### 3. 核心组件

#### 接口层

- `cli.rs` 使用 `clap` 定义命令和全局选项。
- `ui.rs` 渲染终端输出和每条 channel 的健康状态。
- `web.rs` 提供 loopback 状态页和处置命令。
- `main.rs` 实现供 `status`、`stop` 和 `restart` 使用的本地 IPC。

#### 配置流水线

- `config.rs` 解析 TOML 并生成运行时 channel 配置。
- `ssh_config.rs` 解析 OpenSSH alias、默认值、密钥和跳板链。
- `host_check.rs` 为 `hosts` 命令分析 alias 支持状态。
- `port_check.rs` 检查监听地址并探测运行中的隧道。

所有配置都在网络任务启动前完成解析，避免无效 alias、endpoint、认证或跳板环境
进入运行时连接循环。

#### 服务生命周期

`service.rs` 按 SSH 路由分组 channels，持有对应的 `SshManager`，并汇总
`ChannelStatus` 快照。只有目标、端口、用户、认证和 `ProxyJump` 链相同的
channels 才能共享 session；使用相同非零远端端口的远程转发会拆到不同 session。

#### SSH 传输

`ssh.rs` 连接跳板、校验主机密钥、认证目标、注册本地和远程转发、记录健康状态，
并重连路由组。SSH 协议实现由 `russh` 提供。

文件级职责见[模块设计](./modules.md)。

### 4. 数据流

#### 启动

```text
加载 config.toml
  -> 解析 SSH alias 和认证
  -> 校验 endpoint 并解析跳板配置
  -> 分组兼容 channels
  -> 连接并认证每条路由
  -> 启动本地 listener 或注册远程转发
  -> 通过 IPC 和 Web 发布健康状态
```

#### 转发

- `local->remote`：本地 `TcpListener` 接受连接，每个连接打开一个
  `direct-tcpip` SSH channel。
- `remote->local`：客户端注册 `tcpip-forward`，每个 `forwarded-tcpip` 回调连接
  配置的本地 endpoint。

两种方向都以异步方式双向复制数据。

### 5. 并发与状态

- 每个 SSH 路由组在独立 Tokio 任务中运行。
- 兼容 channels 共享一个 SSH session；接受的 TCP 连接使用子任务。
- 全局 permit 串行执行 SSH 握手，避免重连风暴。
- `CancellationToken` 协调进程、IPC、Web、manager 和 listener 的关闭。
- Mutex 只保护短暂的服务和健康状态更新，不跨网络 `.await` 持有。

Channel 健康状态依次表现为 `Stopped`、`Connecting`、`Connected`、
`Reconnecting` 和 `Failed`。`ServiceManager` 对外提供快照，不暴露传输层内部状态。

### 6. 失败与重连

- 配置错误在启动前失败。
- 可重试的传输或 session 错误使用配置的 backoff 和 jitter 重建受影响的路由组。
- 认证和主机密钥失败属于永久错误，直到配置或信任记录发生变化。
- 永久的 channel 级错误会禁用该 channel；条件允许时，兼容 channels 继续运行。
- 可重试的 channel 错误可能重建共享 session 及该路由组的全部 channels。

有限重试轮次耗尽后，使用最高 60 秒的第二层 jitter 指数退避继续恢复；session
成功后两层重试状态都会重置。

### 7. 安全边界

- 目标和跳板都严格校验 `~/.ssh/known_hosts`。
- 未知或已变化的密钥会被拒绝；CLI 和 Web 错误提供处置命令，但不会自动信任。
- 跳板仅支持 public-key 认证，并要求未加密密钥。
- `config.toml` 中的密码和 passphrase 为明文；建议使用密钥认证并限制文件权限。
- IPC 和 Web 状态页仅监听 loopback。
- 可用 SSH 算法以当前 `russh` 配置启用的算法为准。

### 8. 可观测性

结构化 `tracing` 事件包含路由、channel、host、重试和错误上下文。CLI 和 Web 页面
消费相同的 channel 健康状态快照，因此状态语义保持一致。

完整执行时序见[工作流程](./workflow.md)。
