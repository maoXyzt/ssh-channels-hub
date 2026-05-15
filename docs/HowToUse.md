# 使用教程

本文档提供 SSH Channels Hub 的常见使用场景和详细教程。host 信息从 `~/.ssh/config` 读取,`config.toml` 只声明要建立的 channels 以及可选的密码 / passphrase 覆盖。

## 目录

1. [端口转发(本地)](#1-端口转发本地---类-ssh--l)
2. [远程端口转发](#2-远程端口转发类-ssh--r)
3. [常见场景](#3-常见场景)
4. [多 channels 管理](#4-多-channels-管理)
5. [故障排查](#5-故障排查)

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
- `status` 可查看全部 channels 的状态

---

## 5. 故障排查

### 5.1 连接失败

先用原生 ssh 验证 SSH config 本身没问题:

```bash
ssh <alias>                  # 应能成功登录
```

如果原生 ssh 都连不上,问题在 SSH config / 密钥 / 网络,与本工具无关。

### 5.2 端口被占用

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

### 5.3 配置错误

```bash
ssh-channels-hub validate --debug
```

常见错误及解释见 [configuration.md §6](./configuration.md#6-故障排查):

- `references host alias 'X', but no Host X block exists` → SSH config 缺该 alias
- `missing HostName / User` → SSH config 中该 alias 不完整
- `has no IdentityFile ... and no [auth.X].password` → 二选一补全

### 5.4 端口转发不工作

1. 在远程服务器上验证目标服务存在:`curl http://127.0.0.1:8080`
2. `ssh-channels-hub start --debug` 看 SSH 握手与 channel 建立日志
3. 远程转发(`direction = "remote->local"`)无法用 `test` 命令验证,需在服务器端实际连接

---

## 相关文档

- [配置文档](./configuration.md) - 详细字段说明与示例
- [架构设计](./architecture.md) - 系统整体设计
- [工作流程](./workflow.md) - 启动 / 重连 / 关闭流程
