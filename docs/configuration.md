# 配置文档

SSH Channels Hub 把 host 信息(`HostName` / `User` / `Port` / `IdentityFile`)托管给 `~/.ssh/config`。`config.toml` 只保留 channels 定义、可选的密码 / passphrase 覆盖、以及重连策略。

## 1. 配置文件位置

按顺序查找,使用第一个存在的文件:

- 当前目录: `./config.toml`
- Linux/macOS: `~/.config/ssh-channels-hub/config.toml`
- Windows: `%APPDATA%\ssh-channels-hub\config.toml`

`--config` 指定时仅使用该文件,不再查找默认路径。

`~/.ssh/config` 的位置:操作系统默认 `~/.ssh/config`;可在 `config.toml` 顶层用 `ssh_config = "/path/to/config"` 覆盖。

## 2. 文件结构

```toml
# 可选:覆盖 SSH config 路径(默认 ~/.ssh/config)
# ssh_config = "~/.ssh/config"

# Channel 定义(端口转发)
[[channels]]
name      = "db-tunnel"
hostname  = "my-server"           # ← SSH config 里 `Host my-server` 的别名
direction = "local->remote"       # 或 "remote->local"
local     = "3306"                # 本机这一侧的地址,bare port → 127.0.0.1:3306
remote    = "3306"                # 远端这一侧的地址

# 可选:为某个 alias 提供密码 / passphrase(SSH config 存不了)
[auth.my-server]
passphrase = "key-passphrase"

# 重连策略
[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true
```

## 3. 字段说明

### 3.1 顶层

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `ssh_config` | string | `~/.ssh/config` | SSH config 文件路径 |
| `channels` | array | `[]` | channel 定义,见 §3.2 |
| `auth` | table | `{}` | 按 alias 提供密码 / passphrase,见 §3.3 |
| `reconnection` | table | 默认值 | 重连策略,见 §3.5 |

### 3.2 `[[channels]]`

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `name` | string | ✅ | channel 唯一标识 |
| `hostname` | string | ✅ | SSH config 中的 `Host <alias>` 别名 |
| `direction` | string | ✅ | `"local->remote"`(ssh -L) 或 `"remote->local"`(ssh -R) |
| `local` | string | ✅ | 本机这一侧的地址,见下文「Endpoint 格式」 |
| `remote` | string | ✅ | 远端这一侧的地址 |

**字段语义恒定**:`local` 永远表示本机这一侧、`remote` 永远表示 SSH 服务器这一侧 —— 与 `direction` 无关。变动的只是「谁监听 / 谁连接」:

- `direction = "local->remote"`:本机监听 `local`,新连接通过隧道在服务器端打开到 `remote` 的 TCP。流量方向 `本机 local → 服务器 → 远端 remote`。
- `direction = "remote->local"`:服务器在 `remote` 绑定监听,收到连接后桥接到本机 `local`。流量方向 `远端 remote → 服务器 → 本机 local`。

**Endpoint 格式**(`local` / `remote` 都按这个解析):

| 写法 | 解析为 |
|---|---|
| `"3306"` | `127.0.0.1:3306` |
| `"127.0.0.1:3306"` | `127.0.0.1:3306` |
| `"0.0.0.0:8080"` | `0.0.0.0:8080`(本侧多网卡;远端配合服务器 `GatewayPorts` 时全网可达) |
| `"db.internal:5432"` | `db.internal:5432`(主机名) |
| `"[::1]:3306"` | `::1:3306`(IPv6 用方括号) |

未识别的字段会直接报错(`deny_unknown_fields`),不会被静默忽略。

### 3.3 `[auth.<alias>]`

仅在 SSH config 本身无法完成认证时填:

- host 是密码登录(SSH config 没有 `IdentityFile`)
- `IdentityFile` 受 passphrase 保护

| 字段 | 类型 | 说明 |
|---|---|---|
| `password` | string | 密码。**优先于** SSH config 的 `IdentityFile` —— 一旦填写就走密码登录 |
| `passphrase` | string | 密钥 passphrase,附在 SSH config 的 `IdentityFile` 上 |

key 是 SSH config 里的 alias 字符串。**没有覆盖需求的 host 不需要任何 `[auth.X]` 块**。

### 3.4 host info 从哪里来

完全从 `~/.ssh/config` 读。这里只列本工具用到的字段(其它 SSH directives 一律忽略):

| SSH directive | 工具如何用 |
|---|---|
| `Host <alias>` | channel 通过 `hostname = "<alias>"` 引用 |
| `HostName` | 必需。runtime 连接的目标主机 |
| `User` | 必需。SSH 用户名 |
| `Port` | 可选,默认 22 |
| `IdentityFile` | 可选。有则走密钥认证;没有则必须配 `[auth.<alias>].password` |
| `Host *` | 通配的默认值会被继承到其它 Host |
| `ProxyJump` | 可选。仅支持「指向 Host 别名」的写法,详见下文「ProxyJump 限制」 |

**不支持**(SSH 客户端有,工具没有):`ProxyCommand` / `ControlMaster` / `Include` / `Match` 等。

**ProxyJump 限制**:

- 值必须是已经在 `~/.ssh/config` 里定义为 `Host <alias>` 的别名,可以用逗号串成多跳(`ProxyJump alpha,beta`)。原始的 `user@host:port` 形式会被拒绝,提示用户先把跳板写成一个 `Host` 块。
- 跳板仅支持 **publickey 认证**。密钥按 ssh 命令的惯例查找:跳板别名显式 `IdentityFile` > `Host *` 全局 `IdentityFile` > 默认路径(`~/.ssh/id_ed25519` → `id_ecdsa` → `id_rsa` → `id_dsa`,取第一个存在的)。
- 跳板的 IdentityFile **不能被 passphrase 加密**(守护进程没法交互输入)。如果是加密的 key,请解密或换一把未加密的 key。
- 跳板会**严格校验** `~/.ssh/known_hosts`:未记录的跳板主机会被拒绝(不做 TOFU 自动追加)。第一次使用前先 `ssh-keyscan` 写入,或手动 `ssh <alias>` 一次让 OpenSSH 帮你写。
- 跳板别名自己的 `ProxyJump` 设置**不会递归生效**:本工具只读取 channel 目标 host 自身的 `ProxyJump` 链,不再深入。如果你的跳板别名也写了 `ProxyJump`,本工具会忽略,直接把它当成最终一跳。如有真的多级跳板需求,请在目标 host 的 `ProxyJump` 里把所有跳板按顺序逗号列出。

### 3.5 `[reconnection]`

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `max_retries` | u32 | 0 | 最大重试次数,0 = 无限 |
| `initial_delay_secs` | u64 | 1 | 第一次重试前的延迟 |
| `max_delay_secs` | u64 | 30 | 重试之间的最大延迟 |
| `use_exponential_backoff` | bool | true | 指数退避(true) / 固定间隔(false) |

## 4. 示例

### 4.1 标准密钥认证(最常见)

`~/.ssh/config`:
```
Host prod-db
  HostName db.example.com
  User dbuser
  IdentityFile ~/.ssh/id_rsa
```

`config.toml`:
```toml
[[channels]]
name      = "db-tunnel"
hostname  = "prod-db"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

不需要 `[auth.prod-db]`。

### 4.2 密码登录的 host

`~/.ssh/config`:
```
Host jumpbox
  HostName jump.example.com
  User admin
  # 没有 IdentityFile
```

`config.toml`:
```toml
[[channels]]
name      = "jumpbox-web"
hostname  = "jumpbox"
direction = "local->remote"
local     = "8080"
remote    = "80"

[auth.jumpbox]
password = "your-password"
```

### 4.3 IdentityFile 有 passphrase

```toml
[[channels]]
name      = "backup-tunnel"
hostname  = "backup"
direction = "local->remote"
local     = "2222"
remote    = "22"

[auth.backup]
passphrase = "key-passphrase"
```

### 4.4 暴露本机端口给服务器(remote->local / ssh -R)

```toml
[[channels]]
name      = "expose-local-web"
hostname  = "jumpbox"
direction = "remote->local"
remote    = "8022"           # 服务器在 127.0.0.1:8022 监听
local     = "80"             # 收到连接桥接到本机 127.0.0.1:80
```

要让服务器在 `0.0.0.0:8022` 上监听(让其它机器也能连),把 `remote` 改成 `"0.0.0.0:8022"`,**并且**服务器 `/etc/ssh/sshd_config` 里必须开 `GatewayPorts yes`(或 `clientspecified`)。

### 4.5 监听到所有网卡

```toml
[[channels]]
name      = "shared-db"
hostname  = "prod-db"
direction = "local->remote"
local     = "0.0.0.0:3306"   # 本机所有网卡都接受连接
remote    = "3306"
```

### 4.6 多 channel 复用同一 alias

```toml
[[channels]]
name      = "db"
hostname  = "prod-server"
direction = "local->remote"
local     = "3306"
remote    = "3306"

[[channels]]
name      = "redis"
hostname  = "prod-server"
direction = "local->remote"
local     = "6379"
remote    = "6379"

[[channels]]
name      = "web"
hostname  = "prod-server"
direction = "local->remote"
local     = "8080"
remote    = "80"
```

三个 channel 共用同一 SSH 连接的 host info(底层每个 channel 仍各自建一条 SSH session,详见 [architecture.md](./architecture.md))。

### 4.7 通过 ProxyJump 访问内网 host

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

`config.toml`:
```toml
[[channels]]
name      = "inner-db-tunnel"
hostname  = "inner-db"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

效果:本机 `127.0.0.1:3306` ⇄ `bastion` ⇄ `10.0.5.20:3306`。多跳就把 `ProxyJump` 写成逗号列表(`ProxyJump dmz-jump,inner-jump`),按从外到内的顺序。

第一次跑之前确认跳板已在 `~/.ssh/known_hosts` 里:

```bash
ssh-keyscan -p 22 bastion.example.com >> ~/.ssh/known_hosts
# 或者
ssh bastion   # 让 OpenSSH 提示你是否信任并写入
```

### 4.8 用 `generate` 生成脚手架

```bash
ssh-channels-hub generate -o config.toml
```

工具扫一遍 `~/.ssh/config`,为每个 alias 输出**注释掉的** `[[channels]]` 模板。取消注释、填上 `local` / `remote` 端口即可。无 `IdentityFile` 的 host 同时会附上 `[auth.<alias>]` 模板。

## 5. 配置验证

```bash
ssh-channels-hub validate --config /path/to/config.toml
```

会做的检查:

- TOML 语法
- 每个 `channels[].hostname` 在 SSH config 里存在
- 该 alias 有 `HostName` 和 `User`
- 该 alias 有 `IdentityFile` 或 `[auth.<alias>].password` 二选一
- `direction` 取值合法
- `local` / `remote` 是合法的 `port` 或 `host:port`
- 如果 channel 走 `ProxyJump`:每一跳的别名都已定义、有 `User` 和 `HostName`、有可用的 publickey(显式 `IdentityFile`、`Host *` 全局,或 `~/.ssh/id_*` 默认 key 其一)
- 如果有 `ProxyJump`,还会做两项环境前置检查:
  - **失败**:跳板的 `IdentityFile` 文件确实不存在(键路径已解析但落不到磁盘)
  - **警告**:跳板主机在 `~/.ssh/known_hosts` 中没有对应记录(运行时会被严格校验拒绝)

输出会列出每个 channel 解析后的 `user@host:port` + 转发参数。

## 6. 故障排查

### `Channel 'X' references host alias 'Y', but no Host Y block exists in ~/.ssh/config`

`channels[].hostname` 写的别名在 SSH config 里没对应 `Host` 块。检查拼写,或在 `~/.ssh/config` 添加该 alias。

### `SSH config Host 'X' is missing HostName` / `User`

alias 的 `Host` 块里缺 `HostName` 或 `User`。补上即可。`User` 可以放在 `Host *` 里被继承。

### `Host 'X' has no IdentityFile in SSH config and no [auth.X].password in config.toml`

二选一:在 `~/.ssh/config` 给该 host 加 `IdentityFile`,或在 `config.toml` 加 `[auth.X] password = "..."`。

### `Channel host 'X' has ProxyJump 'user@host:port' written as a raw target`

ProxyJump 写成了 `user@host:port` 的字面形式。在 `~/.ssh/config` 里为它建一个 `Host <alias>` 块(填好 `HostName / User / Port / IdentityFile`),然后把 ProxyJump 的值换成那个 alias。

### `ProxyJump alias 'X' ... has no IdentityFile and no default key ... exists`

跳板别名没有显式 `IdentityFile`,`Host *` 也没设默认,而 `~/.ssh/` 下没有 `id_ed25519 / id_ecdsa / id_rsa / id_dsa` 任何一个常见 key。给该跳板加 `IdentityFile`,或者把你常用的 key 改成上述任一标准文件名。

### `ProxyJump alias 'X' uses encrypted IdentityFile`

跳板用的 key 被 passphrase 保护,守护进程没法交互输入。要么解密保存(`ssh-keygen -p -f ~/.ssh/id_xxx` 把 passphrase 设为空),要么把 `IdentityFile` 指向一把未加密的 key。终点 host 不受此限,可以用 `[auth.<alias>].passphrase` 提供。

### `ProxyJump host not in known_hosts; refusing`

启动日志里出现这条,说明某个跳板的主机公钥不在 `~/.ssh/known_hosts`。本工具走严格校验,不做 TOFU 自动追加。先手动 `ssh-keyscan -p <port> <host> >> ~/.ssh/known_hosts`,或对跳板 `ssh <alias>` 一次让 OpenSSH 帮你写入。

### `ProxyJump host key changed since last contact`

跳板主机的公钥与 `~/.ssh/known_hosts` 里记录的不一致 —— 可能是 MITM,也可能是跳板真的重装过。**先核实**(找运维确认指纹),确认无误后把 `~/.ssh/known_hosts` 里的旧记录删掉(报错日志会带 line 行号),让它重新被信任。

### `invalid direction '...', expected "local->remote" or "remote->local"`

`direction` 只接受这两个字符串。注意是带箭头的 ASCII (`->`),不是中文箭头或单破折号。

### `Failed to read SSH config at <path>`

文件不存在或没读权限。检查路径(可在 `config.toml` 用 `ssh_config = "..."` 显式指定)。

### 调试

```bash
ssh-channels-hub start --debug --config /path/to/config.toml
```

`--debug` 会打印每个 channel 解析过程、SSH 握手、重试等。
