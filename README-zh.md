# SSH Channels Hub

> [English](./README.md) | 中文

声明式 SSH 隧道工具,自带自动重连。把所有端口转发一次性写进 TOML,启动一个服务,所有隧道一起上线 —— 链路断了会自动重连。

跨平台(Linux、macOS、Windows)。基于 [russh](https://docs.rs/russh) 用 Rust 编写。

## 适用场景

适合下面这种情况:`ssh -L 3306:127.0.0.1:3306 db.example.com` 已经增长到了 *「我现在有五个这种命令,笔记本会睡眠,Wi-Fi 又时不时掉,我想合上盖子再开机时它们全自动回来」*。

- **声明式**:隧道写在 `configs.toml`,不再散落在 shell 历史或终端窗口里。
- **不重复配置 host 信息**:`HostName` / `User` / `Port` / `IdentityFile` 直接从 `~/.ssh/config` 读,这里只引用 alias。
- **自动重连**:每条隧道独立指数退避;一条断了不会拖死其它。
- **两个方向同一套 schema**:`local->remote`(`ssh -L`)和 `remote->local`(`ssh -R`)。
- **前台或 daemon**:`start` 挂在终端,`start -D` 后台运行;`stop` / `restart` / `status` 走 IPC 跟运行中的进程通信。

## 快速开始

**1. 安装**

```bash
cargo install ssh-channels-hub  # 从 crates.io 安装(推荐)
```
cargo build --release           # 二进制位于 target/release/ssh-channels-hub (Windows 为 .exe)
或从源码构建:

```bash
cargo install --path .          # 在仓库 clone 内构建并安装
# 或
cargo build --release           # 二进制位于 target/release/ssh-channels-hub
```

**2. 确保 `~/.ssh/config` 里有目标 host**

```
Host my-db
  HostName db.example.com
  User myuser
  IdentityFile ~/.ssh/id_rsa
```

**3. 在当前目录写 `configs.toml`**

```toml
[[channels]]
name      = "db"
hostname  = "my-db"             # 对应 ~/.ssh/config 里的 alias
direction = "local->remote"     # 等价于 ssh -L
local     = "3306"              # 本机监听 127.0.0.1:3306
remote    = "3306"              # 服务器连接其本网络里的 127.0.0.1:3306
```

**4. 启动**

```bash
ssh-channels-hub start          # Ctrl+C 退出
```

之后 `mysql -h 127.0.0.1 -P 3306` 就走 SSH 隧道了。

> **提示**:`ssh-channels-hub generate -o configs.toml` 会扫一遍 SSH config,为每个 alias 输出一个**注释掉的** `[[channels]]` 模板,取消注释再填端口即可。或者直接 `cp configs.example.toml configs.toml` 用带注释的示例文件起步。

## 配置

`configs.toml` 默认查找顺序(第一个存在的文件生效):

| 平台 | 路径 |
|---|---|
| 当前目录(永远第一个查) | `./configs.toml` |
| Linux / macOS | `~/.config/ssh-channels-hub/config.toml` |
| Windows | `%APPDATA%\ssh-channels-hub\config.toml` |

也可以用 `--config /path/to/file` 显式指定。

### Channel schema

```toml
[[channels]]
name      = "string"                            # 必填,唯一标识
hostname  = "ssh-config-alias"                  # 必填,来自 ~/.ssh/config 的 alias
direction = "local->remote" | "remote->local"   # 必填
local     = "port" | "host:port"                # 必填,本机这一侧的地址
remote    = "port" | "host:port"                # 必填,远端这一侧的地址
```

`local` 和 `remote` 永远代表对应一侧的地址,与 `direction` 无关。`direction` 决定**谁监听**:

- **`local->remote`**(≈ `ssh -L`):本机监听 `local`,新连接由服务器转发到 `remote`。
- **`remote->local`**(≈ `ssh -R`):服务器在 `remote` 监听,流量经隧道桥接到本机 `local`。

Endpoint 接受这些写法:
- `"3306"` → `127.0.0.1:3306`(裸端口,host 默认 loopback)
- `"127.0.0.1:3306"` → 显式写法
- `"0.0.0.0:8080"` → 监听所有网卡
- `"[::1]:3306"` → IPv6

### 凭证

`~/.ssh/config` 存不了密码或 key passphrase。SSH config 本身无法完成认证时,加 `[auth.<alias>]` 块(按 SSH config 里的 alias 作 key):

```toml
[auth.my-db]
password   = "..."          # 密码登录 host(SSH config 里没 IdentityFile)
# 或
passphrase = "..."          # IdentityFile 被 passphrase 保护
```

`password` 优先级高于 `IdentityFile` —— 一旦填了就走密码登录。能靠 SSH config 单独跑通的 host 不需要 `[auth.*]` 块。

### 重连(全局)

```toml
[reconnection]
max_retries             = 0     # 0 = 无限重试
initial_delay_secs      = 1
max_delay_secs          = 30
use_exponential_backoff = true
```

### 更多示例

**监听所有网卡**,让局域网其它机器也能用这个隧道(注意防火墙):

```toml
[[channels]]
name      = "shared-db"
hostname  = "db-server"
direction = "local->remote"
local     = "0.0.0.0:3306"
remote    = "3306"
```

**把本机服务暴露给 SSH 服务器**(`ssh -R`):

```toml
[[channels]]
name      = "expose-local-web"
hostname  = "jumpbox"
direction = "remote->local"
remote    = "8022"              # 服务器在 127.0.0.1:8022 监听
local     = "80"                # 收到的连接桥接到本机 127.0.0.1:80
```

(要让服务器在 `0.0.0.0:8022` 监听让外部可达,需要把 `remote` 改成 `"0.0.0.0:8022"` **并且**在服务器 `sshd_config` 里启用 `GatewayPorts`。)

完整字段说明:[docs/configuration.md](docs/configuration.md)。

## 命令

| 命令 | 作用 |
|---|---|
| `start` | 前台运行(Ctrl+C 停止)。 |
| `start -D` / `--daemon` | 后台 daemon 模式,与终端分离。 |
| `stop` | 通过 IPC 通知运行中的进程优雅退出。 |
| `restart` | 停止当前服务,再以 daemon 重新启动。 |
| `status` | 显示服务状态、已激活/总 channel 数、PID、channel 列表。 |
| `test` | 测试每个 `local->remote` 的本地监听端口,确认隧道是通的。`remote->local` 的 channel 跳过,需要在服务器端实际连接验证。 |
| `validate` | 把每个 channel 对照 `~/.ssh/config` 解析,列出问题。 |
| `generate -o configs.toml` | 根据 SSH config alias 生成 `configs.toml` 脚手架。 |

所有命令都接受 `--config /path/to/configs.toml` 来指向非默认配置文件,以及 `--debug` 打开详细日志。

## 故障排查

- **`Channel '...' references host alias '...', but no Host ... block exists`** —— `hostname` 写错了,或 `~/.ssh/config` 里没有那个 alias。
- **`Address(es) already in use`** —— `local` 地址被别的进程占了。换端口或者先停掉那个进程。`lsof -i :PORT`(Linux/macOS)或 `netstat -ano | findstr :PORT`(Windows)能查到占用方。
- **绑定 < 1024 的端口** —— Linux/macOS 需要 root,Windows 需要管理员。
- **连不上** —— 先 `ssh <alias>` 手工试一下,排除 SSH config / 网络 / key 权限的问题。
- **加密 key 解不开** —— 加 `[auth.<alias>] passphrase = "..."`。
- **完整 debug 日志** —— `ssh-channels-hub start --debug`,会打印每个 channel 的 SSH 握手、channel 打开、重连尝试等。

## 延伸阅读

- [配置参考](docs/configuration.md) —— 每个字段、每个边界情况。
- [使用教程](docs/HowToUse.md) —— 按任务组织的实际场景。
- [架构](docs/architecture.md) —— channel、session、重连之间怎么搭起来。

## 许可证

MIT —— 见 [LICENSE](LICENSE)。
