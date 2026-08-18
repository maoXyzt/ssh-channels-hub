# Usage Guide / 使用教程

> English | [中文](#中文)

## English

This guide covers common SSH Channels Hub workflows. Host details come from
`~/.ssh/config`; `config.toml` only declares channels and optional password or
passphrase overrides.

### Contents

1. [Local port forwarding](#1-local-port-forwarding-like-ssh--l)
2. [Remote port forwarding](#2-remote-port-forwarding-like-ssh--r)
3. [Common scenarios](#3-common-scenarios)
4. [Managing multiple channels](#4-managing-multiple-channels)
5. [Monitoring tunnel health](#5-monitoring-tunnel-health)
6. [Troubleshooting](#6-troubleshooting)

---

### 1. Local port forwarding (like `ssh -L`)

Forward traffic from a local port through SSH to a remote destination. For
example: local `18080` -> remote `127.0.0.1:8080`.

#### 1.1 Configure `~/.ssh/config`

```text
Host remote-server
  HostName your-remote-server.com
  Port 22                   # optional; default: 22
  User your-username
  IdentityFile ~/.ssh/id_rsa
```

If the host has no `IdentityFile` or the key is encrypted, see
[1.4 Passwords and passphrases](#14-passwords-and-passphrases).

#### 1.2 Configure `config.toml`

```toml
[[channels]]
name      = "web-service-tunnel"
hostname  = "remote-server"     # matches Host remote-server in ~/.ssh/config
direction = "local->remote"
local     = "18080"
remote    = "8080"
```

A complete minimal configuration:

```toml
[[channels]]
name      = "web-service-tunnel"
hostname  = "remote-server"
direction = "local->remote"
local     = "18080"
remote    = "8080"

[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true
```

Default `config.toml` search order:

- Linux/macOS: `./config.toml` -> `$XDG_CONFIG_HOME/ssh-channels-hub/config.toml`
  (or `~/.config/ssh-channels-hub/config.toml` when `XDG_CONFIG_HOME` is unset)
- Windows: `./config.toml` -> `%APPDATA%\ssh-channels-hub\config.toml`

Use `--config` to specify another path.

#### 1.3 Start the service

```bash
ssh-channels-hub validate            # validate first
ssh-channels-hub start               # foreground
ssh-channels-hub start -D            # daemon
ssh-channels-hub start --debug       # debug logs
```

Opening `http://localhost:18080` now reaches `127.0.0.1:8080` on the remote
server through SSH.

#### 1.4 Passwords and passphrases

`~/.ssh/config` cannot store passwords or passphrases. Add an
`[auth.<alias>]` table to `config.toml` when needed:

```toml
# Password authentication (no IdentityFile in SSH config)
[auth.remote-server]
password = "your-password"
```

Or, for an encrypted `IdentityFile`:

```toml
[auth.remote-server]
passphrase = "your-key-passphrase"
```

**`password` takes precedence over the SSH config `IdentityFile`.**

---

### 2. Remote port forwarding (like `ssh -R`)

Expose a service on this machine through a port on the SSH server. For example:
server port `8022` -> local `127.0.0.1:80`.

```toml
[[channels]]
name      = "expose-local-web"
hostname  = "remote-server"     # alias from ~/.ssh/config
direction = "remote->local"
remote    = "8022"              # server listens on 127.0.0.1:8022
local     = "80"                # connections reach local 127.0.0.1:80
```

After startup:

1. The client requests `tcpip-forward` and binds port `8022` on the server.
2. Connections to server port `8022` travel through SSH to local port `80`.
3. Test with `curl http://127.0.0.1:8022` on the server. This loopback binding is
   server-local. For external access, set `remote = "0.0.0.0:8022"`, enable
   `GatewayPorts yes` or `clientspecified`, and open the server firewall.

`ssh-channels-hub test` checks local listeners only. Test `remote->local`
channels from the server side.

---

### 3. Common scenarios

#### 3.1 Access a remote database

`~/.ssh/config`:

```text
Host db-server
  HostName db.example.com
  User admin
  IdentityFile ~/.ssh/id_rsa
```

`config.toml`:

```toml
[[channels]]
name      = "mysql-tunnel"
hostname  = "db-server"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

Then connect with `mysql -h 127.0.0.1 -P 3306`.

#### 3.2 Access a remote Web service

`~/.ssh/config`:

```text
Host web-server
  HostName web.example.com
  User deploy
  IdentityFile ~/.ssh/deploy_key
```

`config.toml`:

```toml
[[channels]]
name      = "web-tunnel"
hostname  = "web-server"
direction = "local->remote"
local     = "8080"
remote    = "80"
```

Open `http://localhost:8080` in a browser.

#### 3.3 Access Redis with password authentication

`~/.ssh/config`:

```text
Host redis-server
  HostName redis.example.com
  User redis-user
```

`config.toml`:

```toml
[[channels]]
name      = "redis-tunnel"
hostname  = "redis-server"
direction = "local->remote"
local     = "6379"
remote    = "6379"

[auth.redis-server]
password = "your-password"
```

#### 3.4 Share a tunnel on the LAN

```toml
[[channels]]
name      = "shared-tunnel"
hostname  = "remote-server"
direction = "local->remote"
local     = "0.0.0.0:8080"      # accept connections on every local interface
remote    = "80"
```

Restrict access with a firewall.

#### 3.5 Reach an internal host through `ProxyJump`

Define both the jump host and target as aliases in `~/.ssh/config`, then refer
to the jump alias from the target's `ProxyJump`. Separate multiple jumps with
commas, ordered from outermost to innermost.

`~/.ssh/config`:

```text
Host bastion
  HostName bastion.example.com
  User opsadmin
  IdentityFile ~/.ssh/id_ed25519

Host inner-db
  HostName 10.0.5.20
  User dbuser
  IdentityFile ~/.ssh/id_ed25519
  ProxyJump bastion
```

`config.toml` does not need jump-host details:

```toml
[[channels]]
name      = "inner-db-tunnel"
hostname  = "inner-db"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

Result: local `127.0.0.1:3306` <-> `bastion` <-> `10.0.5.20:3306`.

The tool strictly checks `known_hosts` for targets and jump hosts and does not
add keys with TOFU. Prepare each host first:

```bash
ssh-keyscan -p 22 bastion.example.com >> ~/.ssh/known_hosts
# Or let OpenSSH record it after a manual login
ssh bastion
```

`ssh-channels-hub validate` checks jump-host key files and `known_hosts`
entries before startup.

Limitations (see [configuration.md section 3.4](./configuration.md#34-where-host-information-comes-from)):

- `ProxyJump` values must be defined `Host` aliases, not raw
  `user@host:port` targets.
- Jump hosts support public-key authentication only, and their `IdentityFile`
  must not be passphrase-protected.
- A jump alias's own `ProxyJump` is not followed recursively. Put the complete
  ordered jump list on the target host.

---

### 4. Managing multiple channels

```toml
# server1 and server2 are defined in ~/.ssh/config

[[channels]]
name      = "db-tunnel"
hostname  = "server1"
direction = "local->remote"
local     = "3306"
remote    = "3306"

[[channels]]
name      = "web-tunnel"
hostname  = "server2"
direction = "local->remote"
local     = "8080"
remote    = "80"

[[channels]]
name      = "redis-tunnel"
hostname  = "server1"           # channels can reuse an alias
direction = "local->remote"
local     = "6379"
remote    = "6379"

# server2 uses password authentication
[auth.server2]
password = "password2"

[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true
```

- Channels are grouped by SSH route at startup.
- Channels with the same target, user, authentication, and `ProxyJump` chain
  share one SSH session and reconnect together.
- Different SSH routes run independently. Handshakes are serialized globally
  to avoid reconnection storms.
- `status` reports each channel's current health.

---

### 5. Monitoring tunnel health

`status` reports each channel's health, not only whether the manager is running:

| Status | Meaning |
|---|---|
| `Connected` | The SSH session is authenticated and the listener or `tcpip-forward` registration is active. |
| `Connecting #n` | Connection attempt number `n` is in progress. |
| `Reconnecting #n` | The previous connection ended and attempt `n` is waiting in backoff; the latest error is included. |
| `Failed` | A permanent authentication, host-key, or channel error stopped retries. |
| `Stopped` | The service has not started or has stopped. |

#### 5.1 One-time status

```bash
ssh-channels-hub status
```

Typical output:

```text
Service Status
  State:         Running
  Channels:      2/3 connected
  Config:        /Users/me/.config/ssh-channels-hub/config.toml
  PID:           34218

  Channels (3):
  db        L->R 127.0.0.1:3306 -> db.internal:3306    Connected
  redis     L->R 127.0.0.1:6379 -> redis.internal:6379 Reconnecting #3
      io error: connection refused (os error 61)
  web       L->R 127.0.0.1:8080 -> web.internal:80     Connected
```

The aggregate row and failed channels are color-coded, and the latest error is
shown below the affected channel.

#### 5.2 Watch mode

```bash
ssh-channels-hub status --watch          # refresh every 2 seconds
ssh-channels-hub status -w -n 5          # every 5 seconds
ssh-channels-hub status -w -n 1          # minimum: every second
```

- ANSI control sequences redraw the status in place.
- `Ctrl+C` exits and restores the terminal.
- If the daemon is not running, watch mode keeps polling. Start it with
  `start -D` in another terminal to observe state changes.
- A one-second interval adds negligible IPC load.

Windows requires an ANSI-capable terminal such as Windows Terminal or
PowerShell 7+.

---

### 6. Troubleshooting

See [Troubleshooting](./troubleshooting.md#english) for connection,
authentication, `known_hosts`, port, and forwarding problems.

---

### Related documentation

- [Configuration reference](./configuration.md) - fields and examples
- [Troubleshooting](./troubleshooting.md#english) - common problems and commands
- [Connection testing](./testing.md) - verify channel operation
- [Architecture](./architecture.md) - system design
- [Workflow](./workflow.md) - startup, reconnection, and shutdown

## 中文

本文档提供 SSH Channels Hub 的常见使用场景和详细教程。host 信息从 `~/.ssh/config` 读取，`config.toml` 只声明要建立的 channels 以及可选的密码 / passphrase 覆盖。

### 目录

1. [端口转发(本地)](#1-端口转发本地---类-ssh--l)
2. [远程端口转发](#2-远程端口转发类-ssh--r)
3. [常见场景](#3-常见场景)
4. [多 channels 管理](#4-多-channels-管理)
5. [实时监控隧道健康](#5-实时监控隧道健康)
6. [故障排查](#6-故障排查)

---

### 1. 端口转发(本地) - 类 `ssh -L`

把本地端口的流量经 SSH 隧道转发到远程目标。例如:本地 `18080` → 远程 `127.0.0.1:8080`。

#### 1.1 配置 `~/.ssh/config`

```
Host remote-server
  HostName your-remote-server.com
  Port 22                   # 可选,默认 22
  User your-username
  IdentityFile ~/.ssh/id_rsa
```

如果该 host 没有 `IdentityFile`(密码登录)，或者 `IdentityFile` 受 passphrase 保护，见 [§1.4 密码与 passphrase](#14-密码与-passphrase)。

#### 1.2 配置 `config.toml`

```toml
[[channels]]
name      = "web-service-tunnel"
hostname  = "remote-server"     # ← 对应 ~/.ssh/config 里的 `Host remote-server`
direction = "local->remote"
local     = "18080"
remote    = "8080"
```

完整最小配置:

```toml
[[channels]]
name      = "web-service-tunnel"
hostname  = "remote-server"
direction = "local->remote"
local     = "18080"
remote    = "8080"

[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true
```

`config.toml` 默认查找顺序:

- Linux/macOS: `./config.toml` → `$XDG_CONFIG_HOME/ssh-channels-hub/config.toml`(未设置时使用 `~/.config/ssh-channels-hub/config.toml`)
- Windows: `./config.toml` → `%APPDATA%\ssh-channels-hub\config.toml`

也可用 `--config` 显式指定。

#### 1.3 启动

```bash
ssh-channels-hub validate            # 先验配置
ssh-channels-hub start               # 前台运行
ssh-channels-hub start -D            # daemon
ssh-channels-hub start --debug       # 调试日志
```

启动后访问 `http://localhost:18080`，流量经 SSH 隧道到 `远程服务器:127.0.0.1:8080`。

#### 1.4 密码与 passphrase

`~/.ssh/config` 存不了密码 / passphrase。需要时在 `config.toml` 加 `[auth.<alias>]`:

```toml
# 密码登录(SSH config 无 IdentityFile)
[auth.remote-server]
password = "your-password"
```

如果 `IdentityFile` 受 passphrase 保护:

```toml
[auth.remote-server]
passphrase = "your-key-passphrase"
```

**`password` 优先于 SSH config 的 `IdentityFile`** —— 一旦填写就走密码登录。

---

### 2. 远程端口转发(类 `ssh -R`)

把**本机**服务暴露到**远程服务器**的端口。例如:服务器 `8022` 端口 → 本机 `127.0.0.1:80`。

```toml
[[channels]]
name      = "expose-local-web"
hostname  = "remote-server"     # ~/.ssh/config 里的别名
direction = "remote->local"
remote    = "8022"              # 服务器在 127.0.0.1:8022 绑定监听
local     = "80"                # 收到连接桥接到本机 127.0.0.1:80
```

启动后:

1. 工具向服务器发 `tcpip-forward`，在服务器上绑定 `8022`
2. 任何对「服务器:8022」的连接，流量经 SSH 隧道到本机 `127.0.0.1:80`
3. 在服务器上用 `curl http://127.0.0.1:8022` 验证。当前 loopback 绑定仅服务器本机
   可访问；如需外部访问，将 `remote` 改为 `"0.0.0.0:8022"`，启用
   `GatewayPorts yes` 或 `clientspecified`，并放行服务器防火墙。

**注意**:`ssh-channels-hub test` 仅测本地监听端口，不验证远程转发，需在服务器侧实际连接验证。

---

### 3. 常见场景

#### 3.1 访问远程数据库

`~/.ssh/config`:
```
Host db-server
  HostName db.example.com
  User admin
  IdentityFile ~/.ssh/id_rsa
```

`config.toml`:
```toml
[[channels]]
name      = "mysql-tunnel"
hostname  = "db-server"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

之后 `mysql -h 127.0.0.1 -P 3306` 即连到远程 MySQL。

#### 3.2 访问远程 Web 服务

`~/.ssh/config`:
```
Host web-server
  HostName web.example.com
  User deploy
  IdentityFile ~/.ssh/deploy_key
```

`config.toml`:
```toml
[[channels]]
name      = "web-tunnel"
hostname  = "web-server"
direction = "local->remote"
local     = "8080"
remote    = "80"
```

浏览器访问 `http://localhost:8080`。

#### 3.3 密码登录的 Redis 服务器

`~/.ssh/config`:
```
Host redis-server
  HostName redis.example.com
  User redis-user
```

`config.toml`:
```toml
[[channels]]
name      = "redis-tunnel"
hostname  = "redis-server"
direction = "local->remote"
local     = "6379"
remote    = "6379"

[auth.redis-server]
password = "your-password"
```

#### 3.4 暴露给局域网

```toml
[[channels]]
name      = "shared-tunnel"
hostname  = "remote-server"
direction = "local->remote"
local     = "0.0.0.0:8080"      # 本机所有网卡都接受连接(默认 127.0.0.1)
remote    = "80"
```

注意防火墙与安全风险。

#### 3.5 通过跳板访问内网 host(ProxyJump)

跳板和目标都写成 `~/.ssh/config` 的 `Host` 别名，在目标的 `ProxyJump` 字段引用跳板别名。多跳就用逗号串起来(顺序从外到内)。

`~/.ssh/config`:
```text
Host bastion
  HostName bastion.example.com
  User opsadmin
  IdentityFile ~/.ssh/id_ed25519

Host inner-db
  HostName 10.0.5.20
  User dbuser
  IdentityFile ~/.ssh/id_ed25519
  ProxyJump bastion
```

`config.toml` 不用感知跳板，跟普通 channel 一样写:

```toml
[[channels]]
name      = "inner-db-tunnel"
hostname  = "inner-db"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

效果:本机 `127.0.0.1:3306` ⇄ `bastion` ⇄ `10.0.5.20:3306`。

**前置准备**(本工具对目标主机和跳板都走严格 `known_hosts` 校验，不做 TOFU 自动追加):

```bash
ssh-keyscan -p 22 bastion.example.com >> ~/.ssh/known_hosts
# 或者一次性手动登入让 OpenSSH 帮你写
ssh bastion
```

`ssh-channels-hub validate` 会前置检查每个跳板的 IdentityFile 是否存在、`known_hosts` 是否已有条目，提早暴露问题。

**限制速览**(完整说明见 [configuration.md §3.4 「ProxyJump 限制」](./configuration.md#34-host-info-从哪里来)):

- `ProxyJump` 的值必须是已经定义的 `Host` 别名，**不接受**原始的 `user@host:port` 形式。
- 跳板仅支持 **publickey 认证**；`IdentityFile` 不能被 passphrase 加密(守护进程没法交互输入)。
- 跳板别名自己的 `ProxyJump` 不会递归生效；要多级跳板，把所有跳板按顺序写进**目标 host** 的 `ProxyJump`。

---

### 4. 多 channels 管理

```toml
# ~/.ssh/config 里已配好 server1 / server2

[[channels]]
name      = "db-tunnel"
hostname  = "server1"
direction = "local->remote"
local     = "3306"
remote    = "3306"

[[channels]]
name      = "web-tunnel"
hostname  = "server2"
direction = "local->remote"
local     = "8080"
remote    = "80"

[[channels]]
name      = "redis-tunnel"
hostname  = "server1"           # 多个 channel 可复用同一 alias
direction = "local->remote"
local     = "6379"
remote    = "6379"

# server2 是密码登录
[auth.server2]
password = "password2"

[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true
```

- 所有 channels 在 `start` 时按 SSH 路由分组
- 目标、用户、认证和 ProxyJump 链一致的 channels 共用一条 SSH session，同组一起重连
- 不同 SSH 路由独立运行；握手全局串行，避免重连风暴
- `status` 可查看每条 channel 的实时健康度(下一节)

---

### 5. 实时监控隧道健康

`status` 会打印每条 channel 的当前健康状态，而不是简单的「manager 是否起来」:

| 状态 | 含义 |
|---|---|
| `Connected` | SSH 会话已认证、本地 listener 已 bind / `tcpip-forward` 已注册 —— 真正在转发流量 |
| `Connecting #n` | 第 n 次连接尝试还在进行(认证、建链) |
| `Reconnecting #n` | 上一次断了，正在 backoff 窗口里等待第 n 次重试；后面会附最近一次失败原因 |
| `Failed` | 认证、Host Key 或 channel 配置等永久错误；不会自动重试 |
| `Stopped` | 没启动或已停止 |

#### 5.1 一次性查看

```bash
ssh-channels-hub status
```

典型输出:

```text
📋  Service Status
  State:         ● Running
  Channels:      2/3 connected
  Config:        /Users/me/.config/ssh-channels-hub/config.toml
  PID:           34218

  Channels (3):
  • db        L→R 127.0.0.1:3306 → db.internal:3306    ● Connected
  • redis     L→R 127.0.0.1:6379 → redis.internal:6379 ● Reconnecting #3
      ↳ io error: connection refused (os error 61)
  • web       L→R 127.0.0.1:8080 → web.internal:80     ● Connected
```

聚合行 `2/3 connected` 全绿 / 部分黄 / 全红会用颜色区分；失败的 channel 会在第二行 dim 显示最近一次失败原因，免去翻 `--debug` 日志的麻烦。

#### 5.2 常驻监控

调试一条不稳定的隧道、或想观察重连恢复过程时:

```bash
ssh-channels-hub status --watch          # 每 2 秒刷新一次
ssh-channels-hub status -w -n 5          # 每 5 秒
ssh-channels-hub status -w -n 1          # 最快每秒
```

行为:

- 用 ANSI 清屏在原地重画，刷新感像 `watch(1)`。
- `Ctrl+C` 退出，终端恢复正常。
- 如果 daemon 还没起来，会一直显示「Service is not running」并持续轮询 —— 你可以在另一个终端 `start -D` 然后回这边看 `Stopped → Connecting → Connected` 的变化。
- 渲染开销可忽略；`-n 1` 也不会给 daemon 造成压力(IPC 是单次 TCP loopback 读写)。

注意:Windows 上需要支持 ANSI 转义的终端(Windows Terminal / PowerShell 7+)，老的 `cmd.exe` 会看到追加输出而不是原地刷新。

---

### 6. 故障排查

常见连接、认证、`known_hosts`、端口和转发问题统一见
[故障排查](./troubleshooting.md#中文)。

---

### 相关文档

- [配置文档](./configuration.md) - 详细字段说明与示例
- [故障排查](./troubleshooting.md#中文) - 常见问题及处置命令
- [连接测试](./testing.md) - 验证 channel 是否正常工作
- [架构设计](./architecture.md) - 系统整体设计
- [工作流程](./workflow.md) - 启动 / 重连 / 关闭流程
