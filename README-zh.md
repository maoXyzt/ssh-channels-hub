# SSH Channels Hub

> [English](./README.md) | 中文

声明式 SSH 隧道工具，自带自动重连。
把所有端口转发一次性写进 TOML，启动一个服务，所有隧道一起上线——链路断开后会自动重连。

跨平台（Linux、macOS、Windows）。基于 [russh](https://docs.rs/russh) 用 Rust 编写。

## 适用场景

适合下面这种情况：`ssh -L 3306:127.0.0.1:3306 db.example.com` 已经增长到了
*「我现在有五个这种命令，笔记本会休眠，Wi-Fi 又时不时掉线，我想合上盖子再打开时它们全都自动恢复」*。

- **声明式**：隧道写在 `config.toml`，不再散落在 shell 历史或终端窗口里。
- **不重复配置 host 信息**：`HostName` / `User` / `Port` / `IdentityFile`
  直接从 `~/.ssh/config` 读取，这里只引用 alias。
- **支持 ProxyJump**：使用 `~/.ssh/config` 里定义好的跳板别名，只接受别名形式，使用公钥认证，并严格校验目标和跳板的 `known_hosts`。详见
  [docs/configuration.md §3.4](docs/configuration.md#34-host-info-从哪里来)。
- **自动重连**：兼容的隧道共用 SSH session；某条 SSH 路由断开时使用带 jitter 的退避策略重连，不影响其他路由。
- **两个方向共用一套 schema**：`local->remote`（`ssh -L`）和 `remote->local`（`ssh -R`）。
- **前台或 daemon**：`start` 在终端运行，`start -D` 在后台运行；`stop` / `restart` / `status` 通过 IPC 与运行中的进程通信。

## 快速开始

**1. 直接运行或安装**

推荐用 `uvx` 直接运行，无需安装：

```bash
uvx ssh-channels-hub --help
```

也可以在已激活的 Python 虚拟环境中用 `pip` 安装：

```bash
pip install ssh-channels-hub
ssh-channels-hub --help
```

Wheel 会安装同一个 `ssh-channels-hub` 二进制，覆盖 Linux x86_64、
macOS ARM64 和 Windows x86_64；运行时不会经过 Python。

如果已经有 Rust/Cargo 工具链，也可以使用以下方式安装：

```bash
cargo binstall ssh-channels-hub          # 需先安装 cargo-binstall；安装预编译二进制
cargo install ssh-channels-hub --locked # 从源码编译并安装
```

开发时可下载源码构建：

```bash
git clone https://github.com/maoXyzt/ssh-channels-hub.git
cd ssh-channels-hub
cargo build --release           # 二进制位于 target/release/ssh-channels-hub（Windows 为 .exe）
```

**2. 确保 `~/.ssh/config` 里有目标 host**

```
Host my-db
  HostName db.example.com
  User myuser
  IdentityFile ~/.ssh/id_rsa
```

**3. 在当前目录写 `config.toml`**

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
uvx ssh-channels-hub start      # 无需安装
# 或在 pip/Cargo 安装后：
ssh-channels-hub start          # Ctrl+C 退出
# 或在 cargo build 后：
./target/release/ssh-channels-hub start       # Linux/macOS
.\target\release\ssh-channels-hub.exe start  # Windows PowerShell
```

之后 `mysql -h 127.0.0.1 -P 3306` 就走 SSH 隧道了。

> **提示**：`ssh-channels-hub generate -o config.toml` 会扫描 SSH config，为每个 alias 输出一个**注释掉的** `[[channels]]` 模板，取消注释后填写端口即可。
> 也可以直接运行 `cp config.example.toml config.toml`，从带注释的示例文件开始配置。

## 配置

`config.toml` 的默认查找顺序如下（第一个存在的文件生效）：

| 平台 | 路径 |
|---|---|
| 当前目录（始终优先） | `./config.toml` |
| Linux / macOS | `~/.config/ssh-channels-hub/config.toml` |
| Windows | `%APPDATA%\ssh-channels-hub\config.toml` |

也可以用 `--config /path/to/file` 显式指定。

### Channel schema

```text
[[channels]]
name      = "string"                            # 必填，唯一标识
hostname  = "ssh-config-alias"                  # 必填，来自 ~/.ssh/config 的 alias
direction = "local->remote" | "remote->local"   # 必填
local     = "port" | "host:port"                # 必填，本机这一侧的地址
remote    = "port" | "host:port"                # 必填，远端这一侧的地址
```

`local` 和 `remote` 始终代表对应一侧的地址，与 `direction` 无关。`direction` 决定**谁监听**：

- **`local->remote`**（≈ `ssh -L`）：本机监听 `local`，新连接由服务器转发到 `remote`。
- **`remote->local`**（≈ `ssh -R`）：服务器监听 `remote`，流量经隧道桥接到本机的 `local`。

Endpoint 接受以下写法：

- `"3306"` → `127.0.0.1:3306`（裸端口，host 默认为 loopback）
- `"127.0.0.1:3306"` → 显式写法
- `"0.0.0.0:8080"` → 监听所有网卡
- `"[::1]:3306"` → IPv6

### Web 状态页

当 `[web].enabled = true`（默认值）时，服务启动后会在 loopback 地址提供实时
channel 控制台，展示服务摘要、每条 channel 的方向、local / remote 端点、健康
状态、重试次数和最近一次错误。前台和 daemon 启动都会打印实际 URL。设置
`[web].enabled = false` 会关闭状态页，也不会打印其 URL。

每条 channel 都有基于 `local` 端点生成的 **Open local** 链接，包括
`remote->local`：链接打开的是被暴露的本地服务，不会使用远端监听地址。

```toml
[web]
enabled = true   # default: true；设为 false 可关闭
port = 9090      # default: 9090；首选端口
strict = false   # default: false；占用时依次尝试 9091、9092……
```

### 凭证

`~/.ssh/config` 无法保存密码或 key passphrase。
仅靠 SSH config 无法完成认证时，添加 `[auth.<alias>]` 块（以 SSH config 中的 alias 作为 key）：

```toml
[auth.my-db]
password   = "..."          # 密码登录 host（SSH config 里没有 IdentityFile）
# 或
passphrase = "..."          # IdentityFile 被 passphrase 保护
```

`password` 的优先级高于 `IdentityFile`。配置后将使用密码认证；仅靠 SSH config 即可完成认证的 host 不需要 `[auth.*]` 块。

### 重连（全局）

```toml
[reconnection]
max_retries             = 0     # 0 = 无限重试
initial_delay_secs      = 1
max_delay_secs          = 30
use_exponential_backoff = true
```

每次重试都会加入 jitter。
有限重试轮次耗尽后仍会自动恢复，但会改用最高 60 秒的第二层指数退避；session 成功建立后，两层计数都会重置。进程内 SSH 握手串行执行，以避免重连风暴。

### 更多示例

#### 与局域网其他设备共享隧道

监听所有网卡，让局域网其他设备也能使用这个隧道（注意防火墙）：

```toml
[[channels]]
name      = "shared-db"
hostname  = "db-server"
direction = "local->remote"
local     = "0.0.0.0:3306"
remote    = "3306"
```

#### 向 SSH 服务器暴露本地网络服务

使用 `remote->local`（`ssh -R`）时，`local` 可以指向本机能够访问的其他服务，不必是 loopback 地址：

```toml
[[channels]]
name      = "lan-api"
hostname  = "edge-server"
direction = "remote->local"
local     = "192.168.1.50:3000" # 只填 "3000" 则代表 127.0.0.1:3000
remote    = "8080"              # edge-server 监听 127.0.0.1:8080
```

这样会把 `edge-server` 上的 `127.0.0.1:8080` 转发到
`192.168.1.50:3000`。

要让服务器监听 `0.0.0.0:8080` 以便外部访问，需要把 `remote` 改成 `"0.0.0.0:8080"`，并在服务器的 `sshd_config` 中设置 `GatewayPorts clientspecified`。

完整字段说明：[docs/configuration.md](docs/configuration.md)。

## 命令

| 命令 | 作用 |
|---|---|
| `start` | 前台运行（按 Ctrl+C 停止）。 |
| `start -D` / `--daemon` | 在后台以 daemon 模式运行，与终端分离。 |
| `stop` | 通过 IPC 通知运行中的进程优雅退出。 |
| `restart` | 停止当前服务，再以 daemon 模式重新启动。 |
| `status` | 显示服务状态、每条 channel 的实时健康状态（Connected / Reconnecting / Failed / Stopped）、PID 和端点信息。添加 `--watch / -w` 可持续刷新，刷新间隔由 `--interval / -n` 控制（单位为秒，默认为 2）。 |
| `test` | 测试每个 `local->remote` 的本地监听端口，确认隧道是否连通。`remote->local` channel 会被跳过，需要在服务器端实际连接以验证。 |
| `validate` | 根据 `~/.ssh/config` 解析每个 channel，并列出问题。 |
| `generate -o config.toml` | 根据 SSH config alias 生成 `config.toml` 脚手架。 |
| `hosts` | 扫描 SSH config alias，显示本工具是否支持每个 host。脚本使用时可添加 `--format json`。 |

所有命令都接受 `--config /path/to/config.toml` 以指定非默认配置文件，也可以使用 `--debug` 打开详细日志。

## 故障排查

- **`Channel '...' references host alias '...', but no Host ... block exists`**——`hostname` 有误，或 `~/.ssh/config` 中没有对应的 alias。
- **`Address(es) already in use`**——`local` 地址已被其他进程占用。更换端口或停止该进程。可以使用 `lsof -i :PORT`（Linux/macOS）或 `netstat -ano | findstr :PORT`（Windows）查找占用方。
- **绑定小于 1024 的端口**——Linux/macOS 需要 root 权限，Windows 需要管理员权限。
- **无法连接**——先手动运行 `ssh <alias>`，排查 SSH config、网络或 key 权限问题。
- **加密 key 无法解锁**——添加 `[auth.<alias>] passphrase = "..."`。
- **完整 debug 日志**——`ssh-channels-hub start --debug` 会打印每个 channel 的 SSH 握手、channel 打开和重连尝试等信息。

## 延伸阅读

- [配置参考](docs/configuration.md)——每个字段和边界情况。
- [使用教程](docs/HowToUse.md)——按任务组织的实际场景。
- [架构](docs/architecture.md)——channel、session 与重连之间如何协作。

## 许可证

MIT——见 [LICENSE](LICENSE)。
