# 使用教程

本文档提供 SSH Channels Hub 的常见使用场景和详细教程。host 信息从 `~/.ssh/config` 读取,`config.toml` 只声明要建立的 channels 以及可选的密码 / passphrase 覆盖。

## 目录

1. [端口转发(本地)](#1-端口转发本地---类-ssh--l)
2. [远程端口转发](#2-远程端口转发类-ssh--r)
3. [常见场景](#3-常见场景)
4. [多 channels 管理](#4-多-channels-管理)
5. [实时监控隧道健康](#5-实时监控隧道健康)
6. [故障排查](#6-故障排查)

---

## 1. 端口转发(本地) - 类 `ssh -L`

把本地端口的流量经 SSH 隧道转发到远程目标。例如:本地 `18080` → 远程 `127.0.0.1:8080`。

### 1.1 配置 `~/.ssh/config`

```
Host remote-server
  HostName your-remote-server.com
  Port 22                   # 可选,默认 22
  User your-username
  IdentityFile ~/.ssh/id_rsa
```

如果该 host 没有 `IdentityFile`(密码登录),或者 `IdentityFile` 受 passphrase 保护,见 [§1.4 密码与 passphrase](#14-密码与-passphrase)。

### 1.2 配置 `config.toml`

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

`config.toml` 默认查找顺序:`./config.toml` → `~/.config/ssh-channels-hub/config.toml` → `%APPDATA%\ssh-channels-hub\config.toml`(Windows)。也可用 `--config` 显式指定。

### 1.3 启动

```bash
ssh-channels-hub validate            # 先验配置
ssh-channels-hub start               # 前台运行
ssh-channels-hub start -D            # daemon
ssh-channels-hub start --debug       # 调试日志
```

启动后访问 `http://localhost:18080`,流量经 SSH 隧道到 `远程服务器:127.0.0.1:8080`。

### 1.4 密码与 passphrase

`~/.ssh/config` 存不了密码 / passphrase。需要时在 `config.toml` 加 `[auth.<alias>]`:

```toml
# 密码登录(SSH config 无 IdentityFile)
[auth.remote-server]
password = "your-password"

# 或:IdentityFile 受 passphrase 保护
[auth.remote-server]
passphrase = "your-key-passphrase"
```

**`password` 优先于 SSH config 的 `IdentityFile`** —— 一旦填写就走密码登录。

---

## 2. 远程端口转发(类 `ssh -R`)

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

1. 工具向服务器发 `tcpip-forward`,在服务器上绑定 `8022`
2. 任何对「服务器:8022」的连接,流量经 SSH 隧道到本机 `127.0.0.1:80`
3. 服务器侧用 `curl http://127.0.0.1:8022` 或从外网访问「服务器公网IP:8022」可验证(外网需服务器防火墙放行)

**注意**:`ssh-channels-hub test` 仅测本地监听端口,不验证远程转发,需在服务器侧实际连接验证。

---

## 3. 常见场景

### 3.1 访问远程数据库

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

### 3.2 访问远程 Web 服务

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

### 3.3 密码登录的 Redis 服务器

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

### 3.4 暴露给局域网

```toml
[[channels]]
name      = "shared-tunnel"
hostname  = "remote-server"
direction = "local->remote"
local     = "0.0.0.0:8080"      # 本机所有网卡都接受连接(默认 127.0.0.1)
remote    = "80"
```

注意防火墙与安全风险。

### 3.5 通过跳板访问内网 host(ProxyJump)

跳板和目标都写成 `~/.ssh/config` 的 `Host` 别名,在目标的 `ProxyJump` 字段引用跳板别名。多跳就用逗号串起来(顺序从外到内)。

`~/.ssh/config`:
```
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

`config.toml` 不用感知跳板,跟普通 channel 一样写:

```toml
[[channels]]
name      = "inner-db-tunnel"
hostname  = "inner-db"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

效果:本机 `127.0.0.1:3306` ⇄ `bastion` ⇄ `10.0.5.20:3306`。

**前置准备**(本工具对跳板走严格 `known_hosts` 校验,不做 TOFU 自动追加):

```bash
ssh-keyscan -p 22 bastion.example.com >> ~/.ssh/known_hosts
# 或者一次性手动登入让 OpenSSH 帮你写
ssh bastion
```

`ssh-channels-hub validate` 会前置检查每个跳板的 IdentityFile 是否存在、`known_hosts` 是否已有条目,提早暴露问题。

**限制速览**(完整说明见 [configuration.md §3.4 「ProxyJump 限制」](./configuration.md#34-host-info-从哪里来)):

- `ProxyJump` 的值必须是已经定义的 `Host` 别名,**不接受**原始的 `user@host:port` 形式。
- 跳板仅支持 **publickey 认证**;`IdentityFile` 不能被 passphrase 加密(守护进程没法交互输入)。
- 跳板别名自己的 `ProxyJump` 不会递归生效;要多级跳板,把所有跳板按顺序写进**目标 host** 的 `ProxyJump`。

---

## 4. 多 channels 管理

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

- 所有 channels 在 `start` 时一起建立,各自独立重连
- 每个 channel 独立建一条 SSH session(即便复用同一 alias)
- `status` 可查看每条 channel 的实时健康度(下一节)

---

## 5. 实时监控隧道健康

`status` 会打印每条 channel 的当前健康状态,而不是简单的「manager 是否起来」:

| 状态 | 含义 |
|---|---|
| `Connected` | SSH 会话已认证、本地 listener 已 bind / `tcpip-forward` 已注册 —— 真正在转发流量 |
| `Connecting #n` | 第 n 次连接尝试还在进行(认证、建链) |
| `Reconnecting #n` | 上一次断了,正在 backoff 窗口里等待第 n 次重试;后面会附最近一次失败原因 |
| `Failed` | 配置的 `max_retries` 用完(只有显式设非 0 才会出现),外层循环 1 秒后会重置成 `Connecting #1` |
| `Stopped` | 没启动或已停止 |

### 5.1 一次性查看

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

聚合行 `2/3 connected` 全绿 / 部分黄 / 全红会用颜色区分;失败的 channel 会在第二行 dim 显示最近一次失败原因,免去翻 `--debug` 日志的麻烦。

### 5.2 常驻监控

调试一条不稳定的隧道、或想观察重连恢复过程时:

```bash
ssh-channels-hub status --watch          # 每 2 秒刷新一次
ssh-channels-hub status -w -n 5          # 每 5 秒
ssh-channels-hub status -w -n 1          # 最快每秒
```

行为:

- 用 ANSI 清屏在原地重画,刷新感像 `watch(1)`。
- `Ctrl+C` 退出,终端恢复正常。
- 如果 daemon 还没起来,会一直显示「Service is not running」并持续轮询 —— 你可以在另一个终端 `start -D` 然后回这边看 `Stopped → Connecting → Connected` 的变化。
- 渲染开销可忽略;`-n 1` 也不会给 daemon 造成压力(IPC 是单次 TCP loopback 读写)。

注意:Windows 上需要支持 ANSI 转义的终端(Windows Terminal / PowerShell 7+),老的 `cmd.exe` 会看到追加输出而不是原地刷新。

---

## 6. 故障排查

### 6.1 连接失败

先用原生 ssh 验证 SSH config 本身没问题:

```bash
ssh <alias>                  # 应能成功登录
```

如果原生 ssh 都连不上,问题在 SSH config / 密钥 / 网络,与本工具无关。

### 6.2 端口被占用

`start` 会预检本地端口。报错例:

```text
Error: Port(s) already in use: 18080, 3306. Please stop the application using these ports or change the configuration.
```

排查:

```bash
# Linux/macOS
lsof -i :18080

# Windows
netstat -ano | findstr :18080
```

### 6.3 配置错误

```bash
ssh-channels-hub validate --debug
```

常见错误及解释见 [configuration.md §6](./configuration.md#6-故障排查):

- `references host alias 'X', but no Host X block exists` → SSH config 缺该 alias
- `missing HostName / User` → SSH config 中该 alias 不完整
- `has no IdentityFile ... and no [auth.X].password` → 二选一补全
- `has ProxyJump 'user@host:port' written as a raw target` → 把跳板写成 `Host` 别名,然后 `ProxyJump` 引用别名
- `ProxyJump alias '...' ... no IdentityFile and no default key ... exists` → 给跳板别名补 `IdentityFile`,或在 `~/.ssh/` 放一个常见名字的 key
- `ProxyJump alias '...' uses encrypted IdentityFile` → 跳板的 key 不能加密;换未加密 key 或解密
- `ProxyJump '... IdentityFile '...' does not exist on disk`(validate 时)→ 路径解析出来了但文件不存在,补上文件或改 `IdentityFile` 指向
- `ProxyJump '...': no entry for ...:port in known_hosts`(validate warning)→ 跑一次 `ssh-keyscan` 或手动 `ssh <alias>` 让 OpenSSH 写入

### 6.4 端口转发不工作

1. 在远程服务器上验证目标服务存在:`curl http://127.0.0.1:8080`
2. `ssh-channels-hub start --debug` 看 SSH 握手与 channel 建立日志
3. 远程转发(`direction = "remote->local"`)无法用 `test` 命令验证,需在服务器端实际连接

---

## 相关文档

- [配置文档](./configuration.md) - 详细字段说明与示例
- [架构设计](./architecture.md) - 系统整体设计
- [工作流程](./workflow.md) - 启动 / 重连 / 关闭流程
