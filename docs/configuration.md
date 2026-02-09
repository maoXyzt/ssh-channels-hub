# 配置文档

## 1. 配置文件格式

SSH Channels Hub 使用 TOML 格式的配置文件。

### 1.1 配置文件位置

默认配置文件按**顺序**查找，使用**第一个存在的文件**：

- **当前目录**: `./configs.toml`
- **Linux/macOS**: `~/.config/ssh-channels-hub/config.toml`
- **Windows**: `%APPDATA%\ssh-channels-hub\config.toml`

**自定义路径**: 使用 `--config` 参数指定时，仅使用该文件，不再查找默认路径。

### 1.2 配置文件结构

```toml
# 重连配置（全局）
[reconnection]
max_retries = 0              # 最大重试次数，0 表示无限重试
initial_delay_secs = 1        # 初始延迟（秒）
max_delay_secs = 30          # 最大延迟（秒）
use_exponential_backoff = true # 使用指数退避

# hosts 定义
[[hosts]]
name = "example-server"
host = "example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "user"

[hosts.auth]
type = "key"                 # 或 "password"
key_path = "~/.ssh/id_rsa"

# channels 定义（端口转发）
[[channels]]
name = "db-tunnel"
hostname = "example-server"  # 引用上面定义的 host name
ports = "3306:3306"          # 格式 "本地端口:远程端口"
# dest_host = "127.0.0.1"    # 可选，默认为 "127.0.0.1"
```

## 2. 配置项详解

### 2.1 重连配置 (`reconnection`)

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `max_retries` | u32 | 0 | 最大重试次数，0 表示无限重试 |
| `initial_delay_secs` | u64 | 1 | 第一次重试前的延迟（秒） |
| `max_delay_secs` | u64 | 30 | 重试之间的最大延迟（秒） |
| `use_exponential_backoff` | bool | true | 是否使用指数退避策略 |

**重连策略说明**:

- **指数退避**: 延迟时间按指数增长，适用于临时性网络故障
- **固定间隔**: 延迟时间固定，适用于周期性检查

### 2.2 hosts 配置

每个 host 是一个数组元素，使用 `[[hosts]]` 定义。

#### 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | host 的唯一标识名称（供 channels 引用） |
| `host` | string | SSH 服务器地址 |
| `username` | string | SSH 用户名 |
| `port` | u16 | 22 | SSH 服务器端口号 |

#### 认证配置 (`auth`)

**重要**: 每个 host 都可以独立配置自己的认证方式，包括使用不同的密钥文件。

支持两种认证方式：

**1. 密码认证**

```toml
[hosts.auth]
type = "password"
password = "your-password"
```

**2. 私钥认证**

```toml
[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"        # 支持 ~ 扩展
passphrase = "optional-passphrase" # 可选，如果密钥有密码保护
```

### 2.3 channels 配置

每个 channel 是一个数组元素，使用 `[[channels]]` 定义。channels 用于定义端口转发（SSH 隧道）。

#### 必需字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | channel 的唯一标识名称 |
| `hostname` | string | 引用的 host 名称（必须匹配 `hosts.name`） |
| `ports` | string | 端口转发，格式见下方（依 `channel_type` 不同） |

#### 可选字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `channel_type` | string | `"direct-tcpip"`（本地转发，类似 ssh -L，默认）或 `"forwarded-tcpip"`（远程转发，类似 ssh -R） |
| `dest_host` | string | direct-tcpip：远程目标地址；forwarded-tcpip：本地连接地址（默认：`127.0.0.1`） |
| `listen_host` | string | 仅 direct-tcpip：本地监听地址（默认：`127.0.0.1`）。填 `"0.0.0.0"` 时接受任意网卡连接 |

**说明**:

- **direct-tcpip**（本地转发，默认）：`ports` 格式为 `"本地端口:远程端口"`，例如 `"8080:80"`。流量：本地端口 → SSH 隧道 → 远程 `dest_host:dest_port`。
- **forwarded-tcpip**（远程转发）：`ports` 格式同样为 `"本地端口:远程端口"`，例如 `"80:8022"`（本地 80 → 远程服务器绑定 8022）。流量：远程服务器端口 → SSH 隧道 → 本地 `dest_host:local_port`。等价于 `ssh -R 8022:127.0.0.1:80`。
- `dest_host` 默认为 `"127.0.0.1"`。
- `listen_host` 仅对 direct-tcpip 有效；设为 `"0.0.0.0"` 时，其他机器可通过本机 IP 访问该端口。

## 3. 配置示例

### 3.1 基本端口转发 channel

```toml
[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true

# hosts 定义
[[hosts]]
name = "db-server"
host = "db.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "user"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

# channels 定义
[[channels]]
name = "db-tunnel"
hostname = "db-server"
ports = "3306:3306"
dest_host = "127.0.0.1"
```

### 3.2 使用密码认证的端口转发

```toml
[[hosts]]
name = "web-server"
host = "web.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "admin"

[hosts.auth]
type = "password"
password = "secure-password"

[[channels]]
name = "web-tunnel"
hostname = "web-server"
ports = "8080:80"
dest_host = "127.0.0.1"
```

### 3.3 使用密钥密码的端口转发

```toml
[[hosts]]
name = "secure-server"
host = "secure.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "user"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"
passphrase = "key-passphrase"

[[channels]]
name = "secure-tunnel"
hostname = "secure-server"
ports = "3306:3306"
dest_host = "127.0.0.1"
```

### 3.3.1 使用非标准 SSH 端口

如果 SSH 服务器使用非标准端口（不是 22），需要显式指定 `port` 字段：

```toml
[[hosts]]
name = "custom-port-server"
host = "example.com"
port = 2222                   # 非标准 SSH 端口
username = "user"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[[channels]]
name = "custom-tunnel"
hostname = "custom-port-server"
ports = "3306:3306"
```

### 3.4 多 channels 配置

```toml
[reconnection]
max_retries = 10
initial_delay_secs = 2
max_delay_secs = 60
use_exponential_backoff = true

# hosts 定义
[[hosts]]
name = "web-server"
host = "web.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "admin"

[hosts.auth]
type = "key"
key_path = "~/.ssh/web_key"

[[hosts]]
name = "db-server"
host = "db.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "dbuser"

[hosts.auth]
type = "password"
password = "db-password"

[[hosts]]
name = "redis-server"
host = "redis.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "redis-user"

[hosts.auth]
type = "key"
key_path = "~/.ssh/redis_key"

# channels 定义
[[channels]]
name = "web-tunnel"
hostname = "web-server"
ports = "8080:80"
dest_host = "127.0.0.1"

[[channels]]
name = "db-tunnel"
hostname = "db-server"
ports = "3306:3306"
dest_host = "127.0.0.1"

[[channels]]
name = "redis-tunnel"
hostname = "redis-server"
ports = "6379:6379"
dest_host = "127.0.0.1"
```

### 3.4.1 远程转发（forwarded-tcpip）配置示例

将本机服务暴露到 SSH 服务器上的端口（类似 `ssh -R`）。`ports` 格式为 **「本地端口:远程端口」**：第一项为本机要连接到的端口，第二项为在服务器上绑定的端口。

```toml
[[hosts]]
name = "jump-server"
host = "jump.example.com"
port = 22
username = "user"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

# 远程转发：服务器上 8022 → 本机 127.0.0.1:80
[[channels]]
name = "expose-local-web"
channel_type = "forwarded-tcpip"
hostname = "jump-server"
ports = "80:8022"             # 本地端口:远程端口（本机 80 ← 服务器 8022）
dest_host = "127.0.0.1"      # 可选，本机要连接到的地址，默认 127.0.0.1
```

- 启动后，程序会向服务器发送 `tcpip-forward`，在服务器上绑定 `8022`。
- 当有人连接「服务器:8022」时，流量经 SSH 隧道转发到本机 `127.0.0.1:80`。

### 3.5 多 hosts 使用不同认证方式

**重要**: 每个 host 都可以独立配置自己的认证方式，包括使用不同的密钥文件。

#### 示例：不同 hosts 使用不同认证方式

```toml
# host 1: 使用密码认证
[[hosts]]
name = "web-server"
host = "web.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "admin"

[hosts.auth]
type = "password"
password = "web-password"

# host 2: 使用默认密钥
[[hosts]]
name = "db-server"
host = "db.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "dbuser"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

# host 3: 使用不同的密钥文件
[[hosts]]
name = "backup-server"
host = "backup.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "backup"

[hosts.auth]
type = "key"
key_path = "~/.ssh/backup_key"
passphrase = "backup-key-passphrase"

# channels 定义
[[channels]]
name = "web-tunnel"
hostname = "web-server"
ports = "8080:80"
dest_host = "127.0.0.1"

[[channels]]
name = "db-tunnel"
hostname = "db-server"
ports = "3306:3306"
dest_host = "127.0.0.1"

[[channels]]
name = "backup-tunnel"
hostname = "backup-server"
ports = "2222:22"
dest_host = "127.0.0.1"
```

#### 使用场景

- **不同服务器使用不同密钥**: 为不同的服务器配置不同的 SSH 密钥，提高安全性
- **混合认证方式**: 某些服务器使用密码，某些使用密钥
- **不同用户账户**: 不同 hosts 可能使用不同的用户名和认证方式
- **密钥管理**: 为不同环境（开发/生产）使用不同的密钥文件

#### 注意事项

1. **密钥文件路径**: 每个 host 的 `key_path` 可以指向不同的密钥文件
2. **密钥密码**: 如果密钥有密码保护，需要在对应 host 的配置中指定 `passphrase`
3. **认证方式独立**: 每个 host 的认证配置完全独立，互不影响
4. **配置灵活性**: 可以根据实际需求为每个 host 选择最合适的认证方式
5. **channel 引用**: channels 通过 `hostname` 字段引用 hosts，确保 `hostname` 与 `hosts.name` 匹配

## 4. 配置验证

使用 `validate` 命令验证配置文件：

```bash
ssh-channels-hub validate --config /path/to/config.toml
```

验证将检查：

- 文件格式是否正确
- 必需字段是否存在
- 字段类型是否正确
- channel 名称是否唯一

## 5. 配置最佳实践

### 5.1 安全性

1. **使用密钥认证而非密码**

   ```toml
   [hosts.auth]
   type = "key"
   key_path = "~/.ssh/id_rsa"
   ```

2. **保护配置文件权限**

   ```bash
   chmod 600 ~/.config/ssh-channels-hub/config.toml
   ```

3. **使用环境变量**（未来功能）
   - 密码可以通过环境变量传递
   - 避免在配置文件中存储敏感信息

### 5.2 性能

1. **合理设置重连参数**
   - 对于稳定的连接，可以增加 `initial_delay_secs`
   - 对于不稳定的连接，使用指数退避

2. **限制重试次数**
   - 生产环境建议设置 `max_retries`
   - 避免无限重试消耗资源

### 5.3 可维护性

1. **使用有意义的 channel 名称**

   ```toml
   name = "production-web-server"  # 好
   name = "channel1"               # 不好
   ```

2. **添加注释**

   ```toml
   # Production database tunnel
   [[hosts]]
   name = "prod-db"
   host = "db.prod.example.com"
   port = 22                    # SSH 端口（可选，默认为 22）
   username = "admin"

   [hosts.auth]
   type = "key"
   key_path = "~/.ssh/prod_key"

   [[channels]]
   name = "prod-db-tunnel"
   hostname = "prod-db"
   ports = "3306:3306"
   dest_host = "127.0.0.1"
   ```

3. **分组管理**
   - 将相关 channels 放在一起
   - 使用注释分隔不同环境

## 6. 配置迁移

### 6.1 版本兼容性

当前版本: `0.1.0`

配置文件格式在主要版本之间可能不兼容。升级时请：

1. 备份现有配置
2. 查看更新日志
3. 根据新格式更新配置

### 6.2 配置转换工具

未来可能提供配置转换工具，帮助迁移旧版本配置。

## 7. 故障排查

### 7.1 常见配置错误

**错误**: `Failed to parse config`

- **原因**: TOML 语法错误
- **解决**: 检查括号、引号是否匹配

**错误**: `Missing required field: hostname`

- **原因**: channel 配置中缺少 `hostname` 字段
- **解决**: 检查 `[[channels]]` 配置中是否指定了 `hostname`，并确保对应的 `[[hosts]]` 存在

**错误**: `Channel 'xxx' references unknown host 'yyy'`

- **原因**: channel 引用的 host 名称不存在
- **解决**: 确保 `channels.hostname` 与 `hosts.name` 匹配

### 7.2 调试配置

使用 `--debug` 标志获取详细日志：

```bash
ssh-channels-hub start --debug --config /path/to/config.toml
```

这将显示：

- 配置加载过程
- 每个字段的解析结果
- 验证错误详情

## 8. 扩展阅读

### 8.1 端口转发说明

当前版本支持两种端口转发类型：

- **direct-tcpip**（本地转发，默认）：本地监听端口，经 SSH 隧道转发到远程目标。等价于 `ssh -L`。
- **forwarded-tcpip**（远程转发）：在 SSH 服务器上绑定端口，将连接经隧道转发到本机地址。等价于 `ssh -R`。

#### 多 channels 配置

在实际应用中，你可以配置多个端口转发 channels：

```toml
[[hosts]]
name = "db-server"
host = "db.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "admin"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[[hosts]]
name = "web-server"
host = "web.example.com"
port = 22                    # SSH 端口（可选，默认为 22）
username = "admin"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

# 数据库端口转发
[[channels]]
name = "db-tunnel"
hostname = "db-server"
ports = "3306:3306"
dest_host = "127.0.0.1"

# Web 服务端口转发
[[channels]]
name = "web-tunnel"
hostname = "web-server"
ports = "8080:80"
dest_host = "127.0.0.1"
```

这样可以在一个配置中同时管理多个端口转发 channels。
