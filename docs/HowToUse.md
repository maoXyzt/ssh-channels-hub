# 使用教程

本文档提供 SSH Channels Hub 的常见使用场景和详细教程。

## 目录

1. [端口转发（SSH 隧道）](#端口转发ssh-隧道)
2. [监控远程日志](#监控远程日志)
3. [执行远程命令](#执行远程命令)
4. [多 channels 管理](#多-channels-管理)

---

## 端口转发（SSH 隧道）

### 场景说明

当你需要在本地访问远程服务器上的服务时，可以使用 SSH 端口转发功能。例如：

- 远程服务器上有一个监听在 `8080` 端口的 Web 服务
- 你希望通过本地 `18080` 端口访问这个服务
- 流量将通过 SSH 隧道安全地转发

### 配置步骤

#### 1. 定义 hosts

首先在配置文件中定义一个 host：

```toml
[[hosts]]
name = "remote-server"
host = "your-remote-server.com"  # 远程服务器地址
port = 22                         # SSH 端口（可选，默认为 22）
username = "your-username"        # SSH 用户名

[hosts.auth]
type = "key"                      # 认证方式：key 或 password
key_path = "~/.ssh/id_rsa"        # SSH 私钥路径
# 如果密钥有密码保护，可以添加：
# passphrase = "your-key-passphrase"
```

**使用密码认证的示例**：

```toml
[[hosts]]
name = "remote-server"
host = "your-remote-server.com"
port = 22                         # SSH 端口（可选，默认为 22）
username = "your-username"

[hosts.auth]
type = "password"
password = "your-password"
```

#### 2. 定义 channels

然后定义一个 channel 来实现端口转发：

```toml
[[channels]]
name = "web-service-tunnel"       # channel 名称（唯一标识）
hostname = "remote-server"         # 引用上面定义的 host name
local_port = 18080                # 本地端口（可选，你访问的端口）
dest_port = 8080                  # 远程服务器上的目标端口
# dest_host = "127.0.0.1"         # 可选，默认为 "127.0.0.1"
```

### 完整配置示例

```toml
# 重连配置（全局）
[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true

# hosts 定义
[[hosts]]
name = "remote-server"
host = "example.com"
port = 22                         # SSH 端口（可选，默认为 22）
username = "user"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

# channels 定义
[[channels]]
name = "web-service-tunnel"
hostname = "remote-server"
local_port = 18080
dest_port = 8080
# dest_host = "127.0.0.1"  # 可选，默认为 "127.0.0.1"
```

### 使用方法

1. **保存配置文件**

   将上述配置保存到默认配置文件位置：
   - **Linux/macOS**: `~/.config/ssh-channels-hub/config.toml`
   - **Windows**: `%APPDATA%\ssh-channels-hub\config.toml`

   或使用自定义路径，通过 `--config` 参数指定。

2. **验证配置**

   ```bash
   ssh-channels-hub validate
   ```

   或验证指定配置文件：

   ```bash
   ssh-channels-hub validate --config /path/to/config.toml
   ```

3. **启动服务**

   ```bash
   ssh-channels-hub start
   ```

   前台运行（用于调试）：

   ```bash
   ssh-channels-hub start --foreground
   ```

   启用调试日志：

   ```bash
   ssh-channels-hub start --debug
   ```

4. **访问服务**

   启动成功后，在浏览器或客户端中访问：

   ```text
   http://localhost:18080
   ```

   流量将通过 SSH 隧道转发到远程服务器的 `127.0.0.1:8080`。

5. **检查状态**

   ```bash
   ssh-channels-hub status
   ```

6. **停止服务**

   ```bash
   ssh-channels-hub stop
   ```

### 工作原理

端口转发的工作原理：

```text
本地应用 → localhost:18080 → SSH 隧道 → 远程服务器:127.0.0.1:8080
```

- **本地端口** (`local_port`): 本地监听的端口，客户端连接此端口（可选）
- **目标端口** (`dest_port`): 远程服务器上服务监听的端口（必需）
- **目标地址** (`dest_host`): 远程服务器上的目标地址，默认为 `127.0.0.1`（可选）

### 注意事项

1. **服务监听地址**

   确保远程服务器上的服务监听在正确的地址：
   - 如果服务监听在 `127.0.0.1:8080`，`dest_host` 使用 `"127.0.0.1"`
   - 如果服务监听在 `0.0.0.0:8080`，`dest_host` 仍使用 `"127.0.0.1"` 即可

2. **端口占用**

   **自动检查**: 服务启动前会自动检查所有配置的本地端口是否被占用。如果检测到端口已被占用，服务将不会启动，并显示明确的错误信息，例如：

   ```text
   Error: Port(s) already in use: 18080, 3306. Please stop the application using these ports or change the configuration.
   ```

   如果遇到端口占用错误，可以：
   - 更换为其他端口（如 `18081`、`18082` 等）
   - 或停止占用该端口的程序
   - 手动检查端口占用情况：

   ```bash
   # Linux/macOS
   lsof -i :18080
   # 或
   netstat -an | grep 18080

   # Windows
   netstat -ano | findstr :18080
   ```

3. **SSH 权限**

   确保 SSH 用户有权限访问远程服务器，并且能够建立 SSH 连接。

4. **防火墙**

   确保本地防火墙允许监听 `local_port` 端口。

### 常见使用场景

#### 场景 1: 访问远程数据库

```toml
[[hosts]]
name = "db-server"
host = "db.example.com"
username = "admin"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[[channels]]
name = "mysql-tunnel"
hostname = "db-server"
local_port = 3306
dest_host = "127.0.0.1"
dest_port = 3306
```

然后可以使用 MySQL 客户端连接 `localhost:3306`。

#### 场景 2: 访问远程 Web 服务

```toml
[[hosts]]
name = "web-server"
host = "web.example.com"
username = "deploy"

[hosts.auth]
type = "key"
key_path = "~/.ssh/deploy_key"

[[channels]]
name = "web-tunnel"
hostname = "web-server"
local_port = 8080
dest_host = "127.0.0.1"
dest_port = 80
```

访问 `http://localhost:8080` 即可访问远程服务器的 Web 服务。

#### 场景 3: 访问远程 Redis

```toml
[[hosts]]
name = "redis-server"
host = "redis.example.com"
username = "redis-user"

[hosts.auth]
type = "password"
password = "your-password"

[[channels]]
name = "redis-tunnel"
hostname = "redis-server"
local_port = 6379
dest_host = "127.0.0.1"
dest_port = 6379
```

---

## 监控远程日志

### 日志监控场景

需要实时监控远程服务器上的日志文件，例如应用日志、系统日志等。

### 日志监控配置

```toml
[[hosts]]
name = "app-server"
host = "app.example.com"
username = "admin"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[[channels]]
name = "app-logs"
hostname = "app-server"
local_port = 0  # 不需要本地端口
dest_host = "127.0.0.1"
dest_port = 0  # 不需要目标端口
```

**注意**: 对于日志监控，实际上应该使用 `session` 类型的 channel，而不是 `direct-tcpip`。当前配置系统主要支持端口转发场景。日志监控功能可能需要使用其他工具或等待后续功能支持。

---

## 执行远程命令

### 命令执行场景

需要在远程服务器上执行命令并查看输出。

**注意**: 当前版本的配置系统主要支持端口转发场景。执行远程命令的功能可能需要使用其他工具或等待后续功能支持。

---

## 多 channels 管理

### 多 channels 场景

同时管理多个 SSH 连接和端口转发。

### 多 channels 配置

```toml
[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true

# 定义多个 hosts
[[hosts]]
name = "server1"
host = "server1.example.com"
username = "user1"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[[hosts]]
name = "server2"
host = "server2.example.com"
username = "user2"

[hosts.auth]
type = "password"
password = "password2"

# 定义多个 channels
[[channels]]
name = "db-tunnel"
hostname = "server1"
local_port = 3306
dest_host = "127.0.0.1"
dest_port = 3306

[[channels]]
name = "web-tunnel"
hostname = "server2"
local_port = 8080
dest_host = "127.0.0.1"
dest_port = 80

[[channels]]
name = "redis-tunnel"
hostname = "server1"
local_port = 6379
dest_host = "127.0.0.1"
dest_port = 6379
```

### 使用说明

1. 所有 channels 会在服务启动时同时建立
2. 每个 channel 独立管理，互不影响
3. 如果某个 channel 断开，会自动重连（根据重连配置）
4. 使用 `status` 命令可以查看所有 channels 的状态

---

## 故障排查

### 连接失败

1. **检查 SSH 连接**

   ```bash
   ssh username@hostname -p port
   ```

   确保能够手动建立 SSH 连接。

2. **检查认证信息**

   - 密钥路径是否正确
   - 密钥权限是否正确（应该是 600）
   - 密码是否正确

3. **检查网络连接**

   确保能够访问远程服务器的 SSH 端口。

### 端口转发不工作

1. **检查本地端口**

   服务启动时会自动检查端口占用。如果启动失败并提示端口被占用，请检查：

   ```bash
   # Linux/macOS
   lsof -i :18080

   # Windows
   netstat -ano | findstr :18080
   ```

   如果端口被占用，请停止占用该端口的程序或更换配置中的端口号。

2. **检查远程服务**

   在远程服务器上检查服务是否正常运行：

   ```bash
   # 在远程服务器上执行
   curl http://127.0.0.1:8080
   ```

3. **检查日志**

   使用 `--debug` 参数启动服务，查看详细日志：

   ```bash
   ssh-channels-hub start --debug --foreground
   ```

### 配置错误

使用 `validate` 命令检查配置：

```bash
ssh-channels-hub validate
```

常见错误：

- `host` 字段引用了不存在的 `hosts.name`
- 缺少必需的字段（如 `name`、`host`、`dest_port` 等）
- TOML 格式错误（括号不匹配、引号不匹配等）

---

## 相关文档

- [配置文档](./configuration.md) - 详细的配置说明
- [架构设计](./architecture.md) - 系统架构说明
- [工作流程](./workflow.md) - 应用程序工作流程

---

## 反馈和建议

如果遇到问题或有改进建议，请：

1. 查看 [故障排查](#故障排查) 部分
2. 检查 [配置文档](./configuration.md) 中的详细说明
3. 提交 Issue 或 PR
