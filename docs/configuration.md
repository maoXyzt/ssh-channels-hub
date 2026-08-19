# Configuration Reference / 配置文档

> English | [中文](#中文)

## English

SSH Channels Hub reads host details (`HostName`, `User`, `Port`, and
`IdentityFile`) from `~/.ssh/config`. `config.toml` contains only channel
definitions, optional password or passphrase overrides, reconnection settings,
and Web status page settings.

### 1. Configuration file locations

The first existing file is used:

- Current directory: `./config.toml`
- Linux/macOS: `$XDG_CONFIG_HOME/ssh-channels-hub/config.toml` (defaults to
  `~/.config/ssh-channels-hub/config.toml`)
- Windows: `%APPDATA%\ssh-channels-hub\config.toml`

When `--config` is provided, only that file is used. The SSH config defaults to
`~/.ssh/config`; override it with `ssh_config = "/path/to/config"` at the top
level of `config.toml`.

### 2. File structure

```toml
# Optional: override the SSH config path (default: ~/.ssh/config)
# ssh_config = "~/.ssh/config"

# Port-forwarding channel
[[channels]]
name      = "db-tunnel"
hostname  = "my-server"           # Host my-server in SSH config
direction = "local->remote"       # or "remote->local"
local     = "3306"                # bare port means 127.0.0.1:3306
remote    = "3306"

# Optional password or passphrase for an alias
[auth.my-server]
passphrase = "key-passphrase"

[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true

[web]
enabled = true
port = 9090
strict = false
```

### 3. Fields

#### 3.1 Top level

| Field | Type | Default | Description |
|---|---|---|---|
| `ssh_config` | string | `~/.ssh/config` | SSH config path |
| `channels` | array | `[]` | Channel definitions; see section 3.2 |
| `auth` | table | `{}` | Password/passphrase overrides by alias; see section 3.3 |
| `reconnection` | table | defaults | Reconnection policy; see section 3.5 |
| `web` | table | defaults | Local Web status page; see section 3.6 |

#### 3.2 `[[channels]]`

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Unique channel name |
| `hostname` | string | yes | `Host <alias>` from SSH config |
| `direction` | string | yes | `"local->remote"` (`ssh -L`) or `"remote->local"` (`ssh -R`) |
| `local` | string | yes | Endpoint on this machine |
| `remote` | string | yes | Endpoint on the SSH-server side |

The meanings are fixed: `local` always means this machine and `remote` always
means the SSH-server side. `direction` determines which endpoint listens:

- `local->remote`: this machine listens on `local`; each connection opens a TCP
  connection to `remote` from the server.
- `remote->local`: the server listens on `remote`; each connection is bridged
  to `local` on this machine.

Both endpoints accept these forms:

| Form | Parsed as |
|---|---|
| `"3306"` | `127.0.0.1:3306` |
| `"127.0.0.1:3306"` | `127.0.0.1:3306` |
| `"0.0.0.0:8080"` | Every interface on that side; remote use also requires `GatewayPorts` |
| `"db.internal:5432"` | Hostname and port |
| `"[::1]:3306"` | IPv6 loopback and port |

Unknown fields are rejected instead of silently ignored.

#### 3.3 `[auth.<alias>]`

Use this table only when SSH config cannot complete authentication:

- The host uses a password and has no `IdentityFile`.
- The configured `IdentityFile` is passphrase-protected.

| Field | Type | Description |
|---|---|---|
| `password` | string | Password authentication; takes precedence over `IdentityFile` |
| `passphrase` | string | Passphrase for the alias's `IdentityFile` |

The table key is the SSH config alias. Hosts that need no override require no
`[auth.X]` table.

#### 3.4 Where host information comes from

The following directives are read from `~/.ssh/config`; other directives are
ignored:

| SSH directive | Usage |
|---|---|
| `Host <alias>` | Referenced by `channels[].hostname` |
| `HostName` | Required connection target |
| `User` | Required SSH username |
| `Port` | Optional; default: 22 |
| `IdentityFile` | Optional key; without one, configure `[auth.<alias>].password` |
| `Host *` | Supplies inherited defaults |
| `ProxyJump` | Optional; values must refer to `Host` aliases |

`ControlMaster`, `Include`, `Match`, and similar OpenSSH features are not
supported.

**Limited `ProxyCommand` compatibility:** The tool accepts only
`ProxyCommand ssh <alias> -W %h:%p` and `ssh -W %h:%p <alias>`, treating them as
`ProxyJump <alias>`. Other commands, tools, or additional options are rejected.
Explicit `ProxyJump` takes precedence over `ProxyCommand`.

**`ProxyJump` limitations:**

- Every value must be a defined `Host` alias. Comma-separated multi-hop chains
  are supported; raw `user@host:port` targets are rejected.
- Jump hosts support public-key authentication only. Keys are selected in this
  order: alias `IdentityFile`, inherited `Host *` `IdentityFile`, or exactly one
  default key under `~/.ssh/`. If several default keys exist, configure one
  explicitly.
- A jump-host `IdentityFile` cannot be passphrase-protected because the daemon
  cannot prompt interactively.
- Targets and jump hosts are strictly checked against `~/.ssh/known_hosts`.
  Unknown keys are rejected; record verified keys with `ssh-keyscan` or a
  manual OpenSSH connection.
- A jump alias's own `ProxyJump` is not followed recursively. Put the complete
  ordered chain on the channel target.

#### 3.5 `[reconnection]`

| Field | Type | Default | Description |
|---|---|---|---|
| `max_retries` | u32 | 0 | Maximum retries per cycle; 0 means unlimited |
| `initial_delay_secs` | u64 | 1 | Delay before the first retry |
| `max_delay_secs` | u64 | 30 | Maximum delay between retries |
| `use_exponential_backoff` | bool | true | Exponential backoff or fixed interval |

Normal retries include jitter. After a finite retry cycle is exhausted, a
second jittered exponential backoff, capped at 60 seconds, starts a new cycle.
A successful session resets both counters. SSH handshakes are serialized within
the process to avoid reconnection storms.

#### 3.6 `[web]`

| Field | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | true | Start the Web status page |
| `port` | u16 | 9090 | Preferred port |
| `strict` | bool | false | Fail if the port is occupied; otherwise try subsequent ports |

The status page listens only on `127.0.0.1`, and startup prints its final URL.
It shows channel directions, endpoints, health, and host-key remediation
commands. **Open local** always uses the `local` endpoint.

Local `local->remote` listeners take priority. Duplicate listeners are rejected
before SSH startup; wildcard addresses (`0.0.0.0` / `::`) conflict with every
address in their family. When Web `strict = false`, its port advances past
channel listeners; with `strict = true`, such a conflict fails validation.
`remote->local` channels bind remotely and are not included in this check.
Different concrete IPs and address families may share a port. Hostnames are
compared case-insensitively without DNS resolution.

### 4. Examples

#### 4.1 Standard key authentication

`~/.ssh/config`:

```text
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

No `[auth.prod-db]` table is needed.

#### 4.2 Password-authenticated host

`~/.ssh/config`:

```text
Host jumpbox
  HostName jump.example.com
  User admin
  # No IdentityFile
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

#### 4.3 Passphrase-protected `IdentityFile`

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

#### 4.4 Expose a local port to the server (`remote->local` / `ssh -R`)

```toml
[[channels]]
name      = "expose-local-web"
hostname  = "jumpbox"
direction = "remote->local"
remote    = "8022"           # server listens on 127.0.0.1:8022
local     = "80"             # forwards to local 127.0.0.1:80
```

To listen on server address `0.0.0.0:8022`, set that value in `remote` and
enable `GatewayPorts yes` or `clientspecified` in the server's `sshd_config`.

#### 4.5 Listen on every local interface

```toml
[[channels]]
name      = "shared-db"
hostname  = "prod-db"
direction = "local->remote"
local     = "0.0.0.0:3306"
remote    = "3306"
```

#### 4.6 Reuse one alias across channels

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

Channels with the same target, user, authentication, and `ProxyJump` chain
share one SSH session and reconnect together. Remote forwards using the same
nonzero remote port are placed in separate sessions to keep callback routing
unambiguous.

#### 4.7 Reach an internal host through `ProxyJump`

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

`config.toml`:

```toml
[[channels]]
name      = "inner-db-tunnel"
hostname  = "inner-db"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

Result: local `127.0.0.1:3306` <-> `bastion` <-> `10.0.5.20:3306`. For multiple
jumps, use an outer-to-inner list such as `ProxyJump dmz-jump,inner-jump`.

Record verified target and jump-host keys before starting:

```bash
ssh-keyscan -p 22 bastion.example.com >> ~/.ssh/known_hosts
# Or let OpenSSH verify and record it
ssh bastion
```

#### 4.8 Generate a scaffold

```bash
ssh-channels-hub generate -o config.toml
```

The command scans SSH config and emits commented `[[channels]]` templates. It
also includes `[auth.<alias>]` templates for hosts without `IdentityFile`.

#### 4.9 Check SSH host support

```bash
ssh-channels-hub hosts
ssh-channels-hub hosts --ssh-config /path/to/ssh_config
ssh-channels-hub hosts --format json
```

`hosts` reports whether each SSH config alias is usable. JSON output contains
`alias`, `hostname`, `status`, `reasons`, and `warnings`.

- `supported`: `HostName` and `User` exist, target authentication can be
  configured, and the `ProxyJump` chain is supported.
- `unsupported`: required fields, aliases, or jump-host keys are missing, or a
  raw jump target is used.
- A target without `IdentityFile` produces a warning because its channel must
  provide `[auth.<alias>].password`.

### 5. Configuration validation

```bash
ssh-channels-hub validate --config /path/to/config.toml
```

Validation checks:

- TOML syntax
- Every channel alias exists and supplies `HostName` and `User`
- Each target has `IdentityFile` or `[auth.<alias>].password`
- `direction` and endpoint formats are valid
- Every `ProxyJump` alias exists, contains required fields, and has a usable
  public key
- Explicit jump-host key files exist; missing `known_hosts` records produce a
  warning because runtime verification will reject them

Output includes each resolved `user@host:port` and forwarding endpoint.

### 6. Configuration error reference

For general problems, see [Troubleshooting](./troubleshooting.md). This
section explains specific validation and startup errors.

#### `Channel 'X' references host alias 'Y', but no Host Y block exists in ~/.ssh/config`

The alias in `channels[].hostname` has no matching SSH config `Host` block.

#### `SSH config Host 'X' is missing HostName` / `User`

Add the missing directive. `User` may be inherited from `Host *`.

#### `Host 'X' has no IdentityFile in SSH config and no [auth.X].password in config.toml`

Add either `IdentityFile` to the SSH config alias or `[auth.X].password` to
`config.toml`.

#### `Channel host 'X' has 'ProxyCommand ...' which this tool does not understand`

Only `ssh <alias> -W %h:%p` and `ssh -W %h:%p <alias>` are accepted. Prefer
OpenSSH 7.3+ and `ProxyJump <alias>` for other cases.

#### `Channel host 'X' has ProxyJump 'user@host:port' written as a raw target`

Define the jump target as an SSH config `Host` alias and reference that alias.

#### `ProxyJump alias 'X' ... has no IdentityFile and no default key ... exists`

Configure `IdentityFile` on the jump alias or provide exactly one standard
default key under `~/.ssh/`.

#### `ProxyJump alias 'X' ... multiple default keys exist`

Set an explicit `IdentityFile`; the tool will not guess among several keys.

#### `ProxyJump alias 'X' uses encrypted IdentityFile`

Use an unencrypted jump-host key. Target hosts may use
`[auth.<alias>].passphrase`.

#### `SSH target key is not trusted; refusing`

Verify the fingerprint, then run the `ssh-keyscan` command shown by the CLI or
Web page.

#### `SSH target host key changed; refusing`

Confirm the change with the administrator, remove the stale entry with a tool
such as `ssh-keygen -R`, and record the new key.

#### `ProxyJump host not in known_hosts; refusing`

Verify the fingerprint, then run the displayed `ssh-keyscan` command or connect
once with OpenSSH.

#### `ProxyJump host key changed since last contact`

Treat the mismatch as a possible man-in-the-middle attack until the change is
verified. Then remove the stale `known_hosts` entry.

#### `invalid direction '...', expected "local->remote" or "remote->local"`

Use one of those two exact ASCII strings.

#### `Failed to read SSH config at <path>`

Check that the file exists and is readable, or set `ssh_config` explicitly.

#### Debugging

```bash
ssh-channels-hub start --debug --config /path/to/config.toml
```

Debug output includes channel resolution, SSH handshakes, and retries.

## 中文

SSH Channels Hub 把 host 信息(`HostName` / `User` / `Port` / `IdentityFile`)托管给 `~/.ssh/config`。`config.toml` 只保留 channels 定义、可选的密码 / passphrase 覆盖、重连策略和 Web 状态页设置。

### 1. 配置文件位置

按顺序查找，使用第一个存在的文件:

- 当前目录: `./config.toml`
- Linux/macOS: `$XDG_CONFIG_HOME/ssh-channels-hub/config.toml`(未设置时默认为 `~/.config/ssh-channels-hub/config.toml`)
- Windows: `%APPDATA%\ssh-channels-hub\config.toml`

`--config` 指定时仅使用该文件，不再查找默认路径。

`~/.ssh/config` 的位置:操作系统默认 `~/.ssh/config`；可在 `config.toml` 顶层用 `ssh_config = "/path/to/config"` 覆盖。

### 2. 文件结构

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

# Web 状态页(以下均为默认值)
[web]
enabled = true
port = 9090
strict = false
```

### 3. 字段说明

#### 3.1 顶层

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `ssh_config` | string | `~/.ssh/config` | SSH config 文件路径 |
| `channels` | array | `[]` | channel 定义，见 §3.2 |
| `auth` | table | `{}` | 按 alias 提供密码 / passphrase，见 §3.3 |
| `reconnection` | table | 默认值 | 重连策略，见 §3.5 |
| `web` | table | 默认值 | 本地 Web 状态页，见 §3.6 |

#### 3.2 `[[channels]]`

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `name` | string | ✅ | channel 唯一标识 |
| `hostname` | string | ✅ | SSH config 中的 `Host <alias>` 别名 |
| `direction` | string | ✅ | `"local->remote"`(ssh -L) 或 `"remote->local"`(ssh -R) |
| `local` | string | ✅ | 本机这一侧的地址，见下文「Endpoint 格式」 |
| `remote` | string | ✅ | 远端这一侧的地址 |

**字段语义恒定**:`local` 永远表示本机这一侧、`remote` 永远表示 SSH 服务器这一侧 —— 与 `direction` 无关。变动的只是「谁监听 / 谁连接」:

- `direction = "local->remote"`:本机监听 `local`，新连接通过隧道在服务器端打开到 `remote` 的 TCP。流量方向 `本机 local → 服务器 → 远端 remote`。
- `direction = "remote->local"`:服务器在 `remote` 绑定监听，收到连接后桥接到本机 `local`。流量方向 `远端 remote → 服务器 → 本机 local`。

**Endpoint 格式**(`local` / `remote` 都按这个解析):

| 写法 | 解析为 |
|---|---|
| `"3306"` | `127.0.0.1:3306` |
| `"127.0.0.1:3306"` | `127.0.0.1:3306` |
| `"0.0.0.0:8080"` | `0.0.0.0:8080`(本侧多网卡；远端配合服务器 `GatewayPorts` 时全网可达) |
| `"db.internal:5432"` | `db.internal:5432`(主机名) |
| `"[::1]:3306"` | `::1:3306`(IPv6 用方括号) |

未识别的字段会直接报错(`deny_unknown_fields`)，不会被静默忽略。

#### 3.3 `[auth.<alias>]`

仅在 SSH config 本身无法完成认证时填:

- host 是密码登录(SSH config 没有 `IdentityFile`)
- `IdentityFile` 受 passphrase 保护

| 字段 | 类型 | 说明 |
|---|---|---|
| `password` | string | 密码。**优先于** SSH config 的 `IdentityFile` —— 一旦填写就走密码登录 |
| `passphrase` | string | 密钥 passphrase，附在 SSH config 的 `IdentityFile` 上 |

key 是 SSH config 里的 alias 字符串。**没有覆盖需求的 host 不需要任何 `[auth.X]` 块**。

#### 3.4 host info 从哪里来

完全从 `~/.ssh/config` 读。这里只列本工具用到的字段(其它 SSH directives 一律忽略):

| SSH directive | 工具如何用 |
|---|---|
| `Host <alias>` | channel 通过 `hostname = "<alias>"` 引用 |
| `HostName` | 必需。runtime 连接的目标主机 |
| `User` | 必需。SSH 用户名 |
| `Port` | 可选，默认 22 |
| `IdentityFile` | 可选。有则走密钥认证；没有则必须配 `[auth.<alias>].password` |
| `Host *` | 通配的默认值会被继承到其它 Host |
| `ProxyJump` | 可选。仅支持「指向 Host 别名」的写法，详见下文「ProxyJump 限制」 |

**不支持**(SSH 客户端有，工具没有):`ControlMaster` / `Include` / `Match` 等。

**`ProxyCommand` 兼容(限定形态)**:OpenSSH 7.3 之前没有 `ProxyJump`，常见做法是 `ProxyCommand ssh <alias> -W %h:%p`。本工具仅识别这一种形态(`ssh -W %h:%p <alias>` 同样接受)，并把它当作 `ProxyJump <alias>` 处理 —— 所有 ProxyJump 限制(见下)同样适用。其它任何 `ProxyCommand` 写法(比如 `nc`、`-o ...` 额外选项、其它 ssh 子命令)一律报错，提示用户升级 OpenSSH 并改用 `ProxyJump`。当 `ProxyJump` 已显式设置时，`ProxyCommand` 被忽略。

**ProxyJump 限制**:

- 值必须是已经在 `~/.ssh/config` 里定义为 `Host <alias>` 的别名，可以用逗号串成多跳(`ProxyJump alpha,beta`)。原始的 `user@host:port` 形式会被拒绝，提示用户先把跳板写成一个 `Host` 块。
- 跳板仅支持 **publickey 认证**。密钥按 ssh 命令的惯例查找:跳板别名显式 `IdentityFile` > `Host *` 全局 `IdentityFile` > 默认路径(`~/.ssh/id_ed25519` / `id_ecdsa` / `id_rsa` / `id_dsa`)。默认路径只在刚好找到一个 key 时自动使用；如果找到多个默认 key，工具会要求你在跳板 `Host` 上显式写 `IdentityFile`，避免选错 key。
- 跳板的 IdentityFile **不能被 passphrase 加密**(守护进程没法交互输入)。如果是加密的 key，请解密或换一把未加密的 key。
- 目标主机和跳板都会**严格校验** `~/.ssh/known_hosts`:未记录的主机会被拒绝(不做 TOFU 自动追加)。第一次使用前先 `ssh-keyscan` 写入，或手动 `ssh <alias>` 一次让 OpenSSH 帮你写。
- 跳板别名自己的 `ProxyJump` 设置**不会递归生效**:本工具只读取 channel 目标 host 自身的 `ProxyJump` 链，不再深入。如果你的跳板别名也写了 `ProxyJump`，本工具会忽略，直接把它当成最终一跳。如有真的多级跳板需求，请在目标 host 的 `ProxyJump` 里把所有跳板按顺序逗号列出。

#### 3.5 `[reconnection]`

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `max_retries` | u32 | 0 | 每轮最大重试次数，0 = 单轮无限 |
| `initial_delay_secs` | u64 | 1 | 第一次重试前的延迟 |
| `max_delay_secs` | u64 | 30 | 重试之间的最大延迟 |
| `use_exponential_backoff` | bool | true | 指数退避(true) / 固定间隔(false) |

每次普通重试都带 jitter。有限重试轮次耗尽后不会每秒重新开始，而是进入另一层带 jitter 的指数退避，上限为 60 秒，再启动新一轮。session 成功建立后连续失败次数和外层退避都会重置。进程内 SSH 握手会串行执行，避免多个连接同时冲击服务器。

#### 3.6 `[web]`

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `enabled` | bool | true | 是否启动 Web 状态页 |
| `port` | u16 | 9090 | 首选监听端口 |
| `strict` | bool | false | true 时端口占用即失败；false 时依次尝试后续端口 |

状态页只监听 `127.0.0.1`。启动时 CLI 会打印最终 URL；若首选端口被占用且
`strict = false`，打印的是实际顺延后绑定的端口。要关闭页面，必须显式设置
`enabled = false`。页面展示每条 channel 的方向、local / remote 端点和实时健康
状态；两种方向的 **Open local** 链接都基于 `local` 地址生成。

本机 `local->remote` 监听优先。重复监听会在 SSH 启动前被拒绝；通配地址
(`0.0.0.0` / `::`)会与同地址族的所有地址冲突。Web 设置 `strict = false` 时会
跳过 channel 已占用的端口，设置 `strict = true` 时冲突会导致校验失败。
`remote->local` 在服务器端监听，不参与此项检查。
不同的具体 IP 和不同地址族可以共用端口；主机名仅按不区分大小写的文本比较，
不会执行 DNS 解析。

### 4. 示例

#### 4.1 标准密钥认证(最常见)

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

#### 4.2 密码登录的 host

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

#### 4.3 IdentityFile 有 passphrase

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

#### 4.4 暴露本机端口给服务器(remote->local / ssh -R)

```toml
[[channels]]
name      = "expose-local-web"
hostname  = "jumpbox"
direction = "remote->local"
remote    = "8022"           # 服务器在 127.0.0.1:8022 监听
local     = "80"             # 收到连接桥接到本机 127.0.0.1:80
```

要让服务器在 `0.0.0.0:8022` 上监听(让其它机器也能连)，把 `remote` 改成 `"0.0.0.0:8022"`，**并且**服务器 `/etc/ssh/sshd_config` 里必须开 `GatewayPorts yes`(或 `clientspecified`)。

#### 4.5 监听到所有网卡

```toml
[[channels]]
name      = "shared-db"
hostname  = "prod-db"
direction = "local->remote"
local     = "0.0.0.0:3306"   # 本机所有网卡都接受连接
remote    = "3306"
```

#### 4.6 多 channel 复用同一 alias

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

三个 channel 的目标、用户、认证和 ProxyJump 链一致，因此共用一条 SSH session。该 session 断开时三个 channel 会一起重连；其他 SSH 路由不受影响。远程转发若使用相同的非零远端端口，会自动拆到不同 session，避免回调路由歧义。

#### 4.7 通过 ProxyJump 访问内网 host

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

`config.toml`:
```toml
[[channels]]
name      = "inner-db-tunnel"
hostname  = "inner-db"
direction = "local->remote"
local     = "3306"
remote    = "3306"
```

效果:本机 `127.0.0.1:3306` ⇄ `bastion` ⇄ `10.0.5.20:3306`。多跳就把 `ProxyJump` 写成逗号列表(`ProxyJump dmz-jump,inner-jump`)，按从外到内的顺序。

第一次跑之前确认目标主机和跳板都已在 `~/.ssh/known_hosts` 里:

```bash
ssh-keyscan -p 22 bastion.example.com >> ~/.ssh/known_hosts
# 或者
ssh bastion   # 让 OpenSSH 提示你是否信任并写入
```

#### 4.8 用 `generate` 生成脚手架

```bash
ssh-channels-hub generate -o config.toml
```

工具扫一遍 `~/.ssh/config`，为每个 alias 输出**注释掉的** `[[channels]]` 模板。取消注释、填上 `local` / `remote` 端口即可。无 `IdentityFile` 的 host 同时会附上 `[auth.<alias>]` 模板。

#### 4.9 检查 SSH host 支持状态

```bash
ssh-channels-hub hosts
ssh-channels-hub hosts --ssh-config /path/to/ssh_config
ssh-channels-hub hosts --format json
```

`hosts` 会列出 SSH config 里的每个 `Host <alias>` 以及本工具是否能使用它。默认读取 `~/.ssh/config`，默认输出面向人工查看的 table；`--format json` 输出稳定 JSON 数组，字段为 `alias`、`hostname`、`status`、`reasons`、`warnings`。

判定规则:

- `supported`:该 alias 有 `HostName` 和 `User`，目标 host 有 `IdentityFile` 或可在实际 channel 配置中补 `[auth.<alias>].password`，且 `ProxyJump` 链符合本工具限制。
- `unsupported`:缺 `HostName`、缺 `User`、`ProxyJump` 写成 `user@host:port` raw target、`ProxyJump` alias 不存在、跳板 alias 缺必要字段或无可用 key。
- 普通目标 host 无 `IdentityFile` 不直接判失败，但会给 warning:实际使用这个 host 的 channel 需要在 `config.toml` 中配置 `[auth.<alias>].password`。

### 5. 配置验证

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
- 如果 channel 走 `ProxyJump`:每一跳的别名都已定义、有 `User` 和 `HostName`、有可用的 publickey(显式 `IdentityFile`、`Host *` 全局，或唯一一个 `~/.ssh/id_*` 默认 key)
- 如果有 `ProxyJump`，还会做两项环境前置检查:
  - **失败**:跳板的 `IdentityFile` 文件确实不存在(键路径已解析但落不到磁盘)
  - **警告**:跳板主机在 `~/.ssh/known_hosts` 中没有对应记录(运行时会被严格校验拒绝)

输出会列出每个 channel 解析后的 `user@host:port` + 转发参数。

### 6. 配置错误参考

通用问题见 [故障排查](./troubleshooting.md)；本节仅解释配置校验和启动时的具体错误。

#### `Channel 'X' references host alias 'Y', but no Host Y block exists in ~/.ssh/config`

`channels[].hostname` 写的别名在 SSH config 里没对应 `Host` 块。检查拼写，或在 `~/.ssh/config` 添加该 alias。

#### `SSH config Host 'X' is missing HostName` / `User`

alias 的 `Host` 块里缺 `HostName` 或 `User`。补上即可。`User` 可以放在 `Host *` 里被继承。

#### `Host 'X' has no IdentityFile in SSH config and no [auth.X].password in config.toml`

二选一:在 `~/.ssh/config` 给该 host 加 `IdentityFile`，或在 `config.toml` 加 `[auth.X] password = "..."`。

#### `Channel host 'X' has 'ProxyCommand ...' which this tool does not understand`

`ProxyCommand` 仅兼容 `ssh <alias> -W %h:%p` 这一种写法(以及参数顺序换一下的 `ssh -W %h:%p <alias>`)。如果你的写法带其它选项(`-q`、`-o ...`)、用其它工具(`nc`、`socat`)或别的 ssh 子命令，本工具不会去解析。最干净的做法是升级到 OpenSSH 7.3+ 并改用 `ProxyJump <alias>`；不能升级的话，把 `ProxyCommand` 改写成上面那一种形态。

#### `Channel host 'X' has ProxyJump 'user@host:port' written as a raw target`

ProxyJump 写成了 `user@host:port` 的字面形式。在 `~/.ssh/config` 里为它建一个 `Host <alias>` 块(填好 `HostName / User / Port / IdentityFile`)，然后把 ProxyJump 的值换成那个 alias。

#### `ProxyJump alias 'X' ... has no IdentityFile and no default key ... exists`

跳板别名没有显式 `IdentityFile`,`Host *` 也没设默认，而 `~/.ssh/` 下没有 `id_ed25519 / id_ecdsa / id_rsa / id_dsa` 任何一个常见 key。给该跳板加 `IdentityFile`，或者把你常用的 key 改成上述任一标准文件名。

#### `ProxyJump alias 'X' ... multiple default keys exist`

跳板别名没有显式 `IdentityFile`，但 `~/.ssh/` 下存在多个默认 key。工具不会猜哪一把 key 应该用于跳板；在跳板的 `Host X` 块里补上明确的 `IdentityFile`。

#### `ProxyJump alias 'X' uses encrypted IdentityFile`

跳板用的 key 被 passphrase 保护，守护进程没法交互输入。要么解密保存(`ssh-keygen -p -f ~/.ssh/id_xxx` 把 passphrase 设为空)，要么把 `IdentityFile` 指向一把未加密的 key。终点 host 不受此限，可以用 `[auth.<alias>].passphrase` 提供。

#### `SSH target key is not trusted; refusing`

目标 host 不在 `~/.ssh/known_hosts` 中。CLI 日志和 Web 状态页会给出
`ssh-keyscan -p <port> <host> >> ~/.ssh/known_hosts`；先核对指纹，确认无误后再执行。

#### `SSH target host key changed; refusing`

目标 host 的主机密钥与已有记录不一致。先核实是否为预期变更，再删除旧记录（例如
`ssh-keygen -R <host>`）并重新写入。

#### `ProxyJump host not in known_hosts; refusing`

启动日志或 Web 状态页里出现这条，说明某个跳板的主机公钥不在 `~/.ssh/known_hosts`。先核对指纹，再执行页面给出的 `ssh-keyscan` 命令，或对跳板 `ssh <alias>` 一次让 OpenSSH 帮你写入。

#### `ProxyJump host key changed since last contact`

跳板主机的公钥与 `~/.ssh/known_hosts` 里记录的不一致 —— 可能是 MITM，也可能是跳板真的重装过。**先核实**(找运维确认指纹)，确认无误后把 `~/.ssh/known_hosts` 里的旧记录删掉(报错日志会带 line 行号)，让它重新被信任。

#### `invalid direction '...', expected "local->remote" or "remote->local"`

`direction` 只接受这两个字符串。注意是带箭头的 ASCII (`->`)，不是中文箭头或单破折号。

#### `Failed to read SSH config at <path>`

文件不存在或没读权限。检查路径(可在 `config.toml` 用 `ssh_config = "..."` 显式指定)。

#### 调试

```bash
ssh-channels-hub start --debug --config /path/to/config.toml
```

`--debug` 会打印每个 channel 解析过程、SSH 握手、重试等。
