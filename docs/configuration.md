# 配置文档

## 1. 配置文件格式

SSH Channels Hub 使用 TOML 格式的配置文件。

### 1.1 配置文件位置

默认配置文件路径：

- **Linux/macOS**: `~/.config/ssh-channels-hub/config.toml`
- **Windows**: `%APPDATA%\ssh-channels-hub\config.toml`
- **自定义**: 使用 `--config` 参数指定

### 1.2 配置文件结构

```toml
# 重连配置（全局）
[reconnection]
max_retries = 0              # 最大重试次数，0 表示无限重试
initial_delay_secs = 1        # 初始延迟（秒）
max_delay_secs = 30          # 最大延迟（秒）
use_exponential_backoff = true # 使用指数退避

# 通道列表
[[channels]]
name = "channel-name"
host = "example.com"
port = 22
username = "user"
channel_type = "session"     # 或 "direct-tcpip"

# 认证配置
[channels.auth]
type = "password"            # 或 "key" 或 "agent"
password = "your-password"

# 通道参数
[channels.params]
command = "tail -f /var/log/app.log"  # 可选，仅用于 session 类型
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

### 2.2 通道配置 (`channels`)

每个通道是一个数组元素，使用 `[[channels]]` 定义。

#### 必需字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 通道的唯一标识名称 |
| `host` | string | SSH 服务器地址 |
| `port` | u16 | SSH 端口，默认 22 |
| `username` | string | SSH 用户名 |
| `channel_type` | string | 通道类型：`session` 或 `direct-tcpip` |

#### 认证配置 (`auth`)

支持三种认证方式：

**1. 密码认证**

```toml
[channels.auth]
type = "password"
password = "your-password"
```

**2. 私钥认证**

```toml
[channels.auth]
type = "key"
key_path = "~/.ssh/id_rsa"        # 支持 ~ 扩展
passphrase = "optional-passphrase" # 可选，如果密钥有密码保护
```

**3. SSH Agent 认证**

```toml
[channels.auth]
type = "agent"
```

*注意: SSH Agent 认证目前尚未完全实现*

#### 通道参数 (`params`)

根据通道类型，参数有所不同：

**Session 通道参数**:

```toml
[channels.params]
command = "tail -f /var/log/app.log"  # 可选，要执行的命令
```

- 如果指定 `command`，将执行该命令
- 如果不指定，将打开一个交互式 shell

**Direct-TCPIP 通道参数**:

```toml
[channels.params]
destination_host = "localhost"    # 必需，目标主机
destination_port = 3306          # 必需，目标端口
```

- 用于端口转发（SSH 隧道）
- 将本地端口转发到远程主机

## 3. 配置示例

### 3.1 基本会话通道

```toml
[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true

[[channels]]
name = "web-server"
host = "web.example.com"
port = 22
username = "admin"
channel_type = "session"

[channels.auth]
type = "key"
key_path = "~/.ssh/id_rsa"
```

### 3.2 执行命令的会话通道

```toml
[[channels]]
name = "log-monitor"
host = "log.example.com"
port = 22
username = "monitor"
channel_type = "session"

[channels.auth]
type = "password"
password = "secure-password"

[channels.params]
command = "tail -f /var/log/application.log"
```

### 3.3 端口转发通道

```toml
[[channels]]
name = "db-tunnel"
host = "db.example.com"
port = 22
username = "user"
channel_type = "direct-tcpip"

[channels.auth]
type = "key"
key_path = "~/.ssh/id_rsa"
passphrase = "key-passphrase"

[channels.params]
destination_host = "localhost"
destination_port = 3306
```

### 3.4 多通道配置

```toml
[reconnection]
max_retries = 10
initial_delay_secs = 2
max_delay_secs = 60
use_exponential_backoff = true

# 通道 1: Web 服务器
[[channels]]
name = "web-1"
host = "web1.example.com"
port = 22
username = "admin"
channel_type = "session"

[channels.auth]
type = "key"
key_path = "~/.ssh/web1_key"

# 通道 2: 数据库隧道
[[channels]]
name = "db-tunnel"
host = "db.example.com"
port = 22
username = "dbuser"
channel_type = "direct-tcpip"

[channels.auth]
type = "password"
password = "db-password"

[channels.params]
destination_host = "localhost"
destination_port = 3306

# 通道 3: 日志监控
[[channels]]
name = "log-monitor"
host = "log.example.com"
port = 2222
username = "monitor"
channel_type = "session"

[channels.auth]
type = "key"
key_path = "~/.ssh/log_key"

[channels.params]
command = "journalctl -f"
```

## 4. 配置验证

使用 `validate` 命令验证配置文件：

```bash
ssh-channels-hub validate --config /path/to/config.toml
```

验证将检查：

- 文件格式是否正确
- 必需字段是否存在
- 字段类型是否正确
- 通道名称是否唯一

## 5. 配置最佳实践

### 5.1 安全性

1. **使用密钥认证而非密码**

   ```toml
   [channels.auth]
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

1. **使用有意义的通道名称**

   ```toml
   name = "production-web-server"  # 好
   name = "channel1"               # 不好
   ```

2. **添加注释**

   ```toml
   # Production database tunnel
   [[channels]]
   name = "prod-db-tunnel"
   # ...
   ```

3. **分组管理**
   - 将相关通道放在一起
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

**错误**: `Missing required field: host`

- **原因**: 缺少必需字段
- **解决**: 检查配置文件中是否所有必需字段都已填写

**错误**: `Invalid channel type: xyz`

- **原因**: 不支持的通道类型
- **解决**: 使用 `session` 或 `direct-tcpip`

### 7.2 调试配置

使用 `--debug` 标志获取详细日志：

```bash
ssh-channels-hub start --debug --config /path/to/config.toml
```

这将显示：

- 配置加载过程
- 每个字段的解析结果
- 验证错误详情
