# 测试 Channel 连接

本文档介绍如何测试 SSH channel 是否成功建立并正常工作。

## 方法 1: 使用内置测试命令（推荐）

项目提供了一个 `test` 命令来验证所有配置的 channels 是否正常工作：

```bash
# 使用默认配置文件
cargo run test

# 使用指定的配置文件
cargo run test -c configs.toml
```

这个命令会：

1. 加载配置文件
2. 尝试连接到每个 channel 配置的本地端口
3. 显示每个 channel 的连接测试结果
4. 提供故障排查建议（如果测试失败）

### 示例输出

**成功的情况：**

```
Testing 1 channel(s)...

Testing channel 'ssc_dev:3923' (local:80 -> 127.0.0.1:3923)... ✓ Connected

✓ All channels are working correctly!
```

**失败的情况：**

```
Testing 1 channel(s)...

Testing channel 'ssc_dev:3923' (local:80 -> 127.0.0.1:3923)... ✗ Failed to connect

✗ Some channels failed the connection test

Troubleshooting tips:
1. Make sure the service is running: cargo run start -c configs.toml
2. Check if ports are listening: netstat -an | grep LISTEN
3. Verify SSH connection is established (check logs with --debug)
4. Ensure remote service is accessible from the SSH server
```

## 方法 2: 手动测试端口连接

### 使用 telnet 或 nc (netcat)

```bash
# Linux/macOS
telnet localhost 80
# 或
nc -zv localhost 80

# Windows
telnet localhost 80
# 或使用 PowerShell
Test-NetConnection -ComputerName localhost -Port 80
```

如果连接成功，说明端口转发正常工作。

### 使用 curl 测试 HTTP 服务

如果转发的是 HTTP 服务：

```bash
curl http://localhost:80
```

### 使用数据库客户端测试数据库连接

如果转发的是数据库端口（如 MySQL）：

```bash
mysql -h 127.0.0.1 -P 80 -u username -p
```

## 方法 3: 检查端口监听状态

### Linux/macOS

```bash
# 检查端口是否在监听
netstat -an | grep LISTEN | grep :80
# 或使用 ss
ss -tlnp | grep :80
# 或使用 lsof
lsof -i :80
```

### Windows

```powershell
# PowerShell
Get-NetTCPConnection -LocalPort 80 -State Listen

# CMD
netstat -ano | findstr :80
```

## 方法 4: 使用调试日志

启动服务时使用 `--debug` 标志查看详细日志：

```bash
cargo run start -c configs.toml --debug
```

日志会显示：

- SSH 连接建立过程
- Channel 打开过程
- 连接错误（如果有）

关键日志信息：

- `"Establishing SSH connection"` - 正在建立 SSH 连接
- `"SSH connection established, authenticating"` - SSH 连接已建立，正在认证
- `"Authentication successful, opening channel"` - 认证成功，正在打开 channel
- `"Direct TCP/IP channel opened"` - Channel 已成功打开

## 方法 5: 检查服务状态

使用 `status` 命令查看服务状态：

```bash
cargo run status -c configs.toml
```

这会显示：

- 服务状态（Running/Stopped/Error）
- 活动的 channel 数量
- 总 channel 数量

## 常见问题排查

### 1. 端口连接失败

**可能原因：**

- 服务未启动
- SSH 连接未建立
- 远程服务不可用
- 防火墙阻止

**解决方法：**

- 确保服务正在运行：`cargo run start -c configs.toml`
- 检查 SSH 连接是否成功（查看日志）
- 验证远程服务是否可以从 SSH 服务器访问
- 检查防火墙设置

### 2. 端口已被占用

**错误信息：**

```
Error: Port(s) already in use: 80
```

**解决方法：**

- 停止占用端口的其他程序
- 或修改配置文件使用其他端口

### 3. SSH 认证失败

**错误信息：**

```
Failed to authenticate: Key authentication failed
```

**解决方法：**

- 检查密钥文件路径是否正确
- 验证密钥文件权限（Linux/macOS: `chmod 600 ~/.ssh/id_rsa`）
- 确认密钥是否需要密码（passphrase）
- 验证 SSH 服务器是否接受该密钥

### 4. Channel 打开失败

**错误信息：**

```
Failed to open direct-tcpip channel
```

**可能原因：**

- 远程目标端口不可访问
- SSH 服务器不允许端口转发
- 目标服务未运行

**解决方法：**

- 验证远程服务是否在运行
- 检查 SSH 服务器配置（`AllowTcpForwarding` 应该为 `yes`）
- 确认目标地址和端口正确

## 最佳实践

1. **启动后立即测试**：服务启动后，立即运行 `test` 命令验证连接
2. **定期检查**：定期运行 `status` 命令检查服务状态
3. **使用调试模式**：遇到问题时，使用 `--debug` 标志获取详细日志
4. **监控日志**：关注日志中的错误和警告信息

## 自动化测试脚本示例

可以创建一个简单的测试脚本：

```bash
#!/bin/bash
# test-channels.sh

CONFIG_FILE="configs.toml"

echo "Starting service..."
cargo run start -c "$CONFIG_FILE" &
SERVICE_PID=$!

# Wait for service to start
sleep 3

echo "Testing channels..."
cargo run test -c "$CONFIG_FILE"

# Cleanup
kill $SERVICE_PID
```
