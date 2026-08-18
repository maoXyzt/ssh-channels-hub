# Workflows / 工作流程文档

> English | [中文](#中文)

## English

### 1. Process startup

```text
Parse CLI
  -> initialize tracing
  -> resolve config.toml and SSH config paths
  -> dispatch the selected command
```

`--debug` selects verbose logging. `--config` disables default config-file
discovery and uses the supplied path.

### 2. Command workflows

#### `start`

```text
Load and resolve configuration
  -> reject occupied local listeners
  -> create ServiceManager
  -> group compatible channels by SSH route
  -> start one SshManager per group
  -> start IPC and, when enabled, the Web status page
  -> write runtime PID and port files
  -> wait for Ctrl+C, IPC stop, or task failure
  -> stop managers and remove runtime files
```

Foreground mode remains attached to the terminal. `start -D` spawns a detached
child that runs the same service workflow.

#### `stop`

The command reads the runtime IPC port, sends `stop`, and waits for the running
process to cancel its tasks and remove runtime files. Stale files are cleaned up
when no process can be reached.

#### `restart`

The command stops the current daemon when present, waits for shutdown, cleans
stale runtime files, and starts a new daemon.

#### `status`

The command queries IPC for the service state and per-channel snapshots. It
prints directions, endpoints, health, retry attempts, and recent failures.
`--watch` repeats the query at the configured interval.

#### `validate`

The command parses TOML, resolves every SSH alias and auth method, validates
directions and endpoints, and performs jump-host key-file and `known_hosts`
preflight checks without starting listeners.

#### `generate`, `hosts`, and `test`

- `generate` turns SSH config aliases into commented channel templates.
- `hosts` reports which aliases are supported and why.
- `test` probes active `local->remote` listeners; `remote->local` channels are
  skipped and must be tested server-side.

### 3. SSH route startup

```text
SshManager::start
  -> mark channels Connecting
  -> acquire the global handshake permit
  -> connect and authenticate each ProxyJump hop
  -> connect to the target through the resulting stream
  -> verify the target host key
  -> authenticate the target
  -> register all channel services on the shared session
```

Host keys are checked against `~/.ssh/known_hosts`. Unknown or changed keys are
permanent failures, and the error includes a remediation command for the CLI
and Web page.

### 4. Forwarding workflows

#### `local->remote` (`ssh -L`)

```text
Bind local TcpListener
  -> mark channel Connected
  -> accept a local connection
  -> open direct-tcpip to the remote endpoint
  -> copy bytes in both directions
  -> close the SSH channel when the stream ends
```

Each accepted TCP connection gets its own SSH channel while sharing the route's
SSH session.

#### `remote->local` (`ssh -R`)

```text
Request tcpip-forward on the server
  -> record the actual remote bind port
  -> mark channel Connected
  -> receive a forwarded-tcpip callback
  -> connect to the configured local endpoint
  -> copy bytes in both directions
```

Remote forwards with conflicting nonzero bind ports are separated into
different sessions so callbacks can be routed unambiguously.

### 5. Reconnection

```text
Retryable connection/session failure
  -> mark channels Reconnecting
  -> wait using configured backoff plus jitter
  -> serialize the next SSH handshake
  -> rebuild the session and all channel services
```

`max_retries = 0` retries indefinitely within one cycle. After a finite cycle
is exhausted, recovery continues with a second jittered exponential backoff
capped at 60 seconds. A successful session resets both levels.

Authentication, host-key, and permanent channel errors are marked `Failed`
instead of being retried.

### 6. Status delivery

Each `SshManager` updates shared per-channel health cells. `ServiceManager`
aggregates snapshots for two consumers:

- IPC serializes status for `status` and `status --watch`.
- The loopback Web server renders the latest snapshot for each request.

### 7. Shutdown

```text
Ctrl+C or IPC stop
  -> cancel the service token
  -> stop each SshManager
  -> cancel listeners and forwarding tasks
  -> close SSH sessions
  -> mark channels Stopped
  -> remove PID, IPC-port, and Web-port files
```

### 8. Concurrency and error boundaries

- Route groups run independently in Tokio tasks.
- Compatible channels share one session; local connections use child tasks.
- SSH handshakes are globally limited to one at a time.
- Locks protect short state updates and are not held across network `.await`s.
- Configuration errors stop startup; a route failure does not stop unrelated
  route groups.

## 中文

### 1. 进程启动

```text
解析 CLI
  -> 初始化 tracing
  -> 确定 config.toml 和 SSH config 路径
  -> 分发所选命令
```

`--debug` 打开详细日志。指定 `--config` 后不再搜索默认配置路径。

### 2. 命令流程

#### `start`

```text
加载并解析配置
  -> 拒绝已占用的本地监听地址
  -> 创建 ServiceManager
  -> 按 SSH 路由分组兼容 channels
  -> 每组启动一个 SshManager
  -> 启动 IPC，以及启用时的 Web 状态页
  -> 写入 PID 和端口运行时文件
  -> 等待 Ctrl+C、IPC stop 或任务失败
  -> 停止 managers 并删除运行时文件
```

前台模式保持连接终端；`start -D` 启动脱离终端的子进程，执行相同的服务流程。

#### `stop`

读取运行时 IPC 端口并发送 `stop`。运行中的进程取消任务、关闭服务并删除运行时
文件；无法连接进程时会清理陈旧文件。

#### `restart`

停止已有 daemon，等待退出并清理陈旧运行时文件，然后启动新的 daemon。

#### `status`

通过 IPC 查询服务状态和每条 channel 的快照，展示方向、endpoint、健康状态、
重试次数和最近错误。`--watch` 按指定间隔重复查询。

#### `validate`

解析 TOML，解析每个 SSH alias 和认证方式，校验 direction 与 endpoint，并在不启动
监听器的情况下检查跳板密钥文件和 `known_hosts`。

#### `generate`、`hosts` 和 `test`

- `generate` 根据 SSH config alias 生成注释状态的 channel 模板。
- `hosts` 报告 alias 是否受支持及原因。
- `test` 探测运行中的 `local->remote` 本地监听；`remote->local` 需在服务器端验证。

### 3. SSH 路由启动

```text
SshManager::start
  -> 将 channels 标记为 Connecting
  -> 获取全局握手许可
  -> 依次连接并认证 ProxyJump 跳板
  -> 通过最终 stream 连接目标
  -> 校验目标主机密钥
  -> 认证目标
  -> 在共享 session 上注册所有 channel 服务
```

目标和跳板均严格校验 `~/.ssh/known_hosts`。未知或已变化的密钥属于永久失败，
错误中包含供 CLI 和 Web 页面展示的处置命令。

### 4. 转发流程

#### `local->remote`（`ssh -L`）

```text
绑定本地 TcpListener
  -> 将 channel 标记为 Connected
  -> 接受本地连接
  -> 向远端 endpoint 打开 direct-tcpip
  -> 双向复制数据
  -> stream 结束时关闭 SSH channel
```

每个 TCP 连接使用独立 SSH channel，但共享该路由的 SSH session。

#### `remote->local`（`ssh -R`）

```text
向服务器请求 tcpip-forward
  -> 记录实际远端绑定端口
  -> 将 channel 标记为 Connected
  -> 接收 forwarded-tcpip 回调
  -> 连接配置的本地 endpoint
  -> 双向复制数据
```

使用相同非零远端端口的冲突转发会拆到不同 session，避免回调路由歧义。

### 5. 重连流程

```text
可重试的连接或 session 失败
  -> 将 channels 标记为 Reconnecting
  -> 按配置的 backoff 和 jitter 等待
  -> 串行执行下一次 SSH 握手
  -> 重建 session 和全部 channel 服务
```

`max_retries = 0` 表示单轮无限重试。有限轮次耗尽后，使用最高 60 秒的第二层
jitter 指数退避继续恢复；session 成功后两层状态都会重置。

认证、主机密钥和永久 channel 错误会标记为 `Failed`，不自动重试。

### 6. 状态传递

每个 `SshManager` 更新共享的 channel 健康状态，`ServiceManager` 汇总快照供两个
消费者使用：

- IPC 为 `status` 和 `status --watch` 序列化状态。
- loopback Web 服务在每次请求时渲染最新快照。

### 7. 关闭流程

```text
Ctrl+C 或 IPC stop
  -> 取消 service token
  -> 停止每个 SshManager
  -> 取消 listener 和转发任务
  -> 关闭 SSH session
  -> 将 channels 标记为 Stopped
  -> 删除 PID、IPC 端口和 Web 端口文件
```

### 8. 并发与错误边界

- SSH 路由组在独立 Tokio 任务中运行。
- 兼容 channels 共享 session；本地连接使用子任务。
- SSH 握手全局限制为单并发。
- 锁只保护短暂状态更新，不跨网络 `.await`。
- 配置错误会阻止启动；单个路由失败不会停止无关路由组。
