# Troubleshooting / 故障排查

> English | [中文](#中文)

## English

### Host key / `known_hosts`

#### Missing host key

The target or a `ProxyJump` host is missing from `~/.ssh/known_hosts`. After
verifying the fingerprint, run the command shown in the CLI log or Web page:

```bash
ssh-keyscan -p <port> <host> >> ~/.ssh/known_hosts
```

#### Changed host key

The key presented by the server no longer matches the existing
`known_hosts` entry. Treat this as a possible man-in-the-middle attack until
the change is confirmed by the server administrator. After verification,
remove the stale entry (for example with `ssh-keygen -R`) and connect once to
record the new key.

### SSH config alias not found

`Channel '...' references host alias '...', but no Host ... block exists` means
`hostname` is misspelled or the alias is missing from `~/.ssh/config`.

### Local port already in use

`Address(es) already in use` means another process owns the configured `local`
address. Change the port or stop that process. Find it with
`lsof -i :PORT` (Linux/macOS) or `netstat -ano | findstr :PORT` (Windows).

### Privileged local port

Binding a port below 1024 requires root (Linux/macOS) or Administrator
permissions (Windows).

### SSH connection or authentication fails

Run `ssh <alias>` manually first to isolate SSH config, network, or key
permission problems.

### Encrypted private key

Set `[auth.<alias>] passphrase = "..."` for an encrypted target key.

### Channel forwarding fails

Verify the destination service from the SSH server and check that the server
allows TCP forwarding. `remote->local` channels must be tested server-side.

### Debug logs

`ssh-channels-hub start --debug` logs each channel's SSH handshake, channel
open, and reconnection attempts.

Detailed validation messages are listed in the
[configuration error reference](./configuration.md#6-配置错误参考).

## 中文

### 主机密钥 / `known_hosts`

#### 缺少主机密钥记录

目标 host 或 `ProxyJump` 跳板不在 `~/.ssh/known_hosts` 中。核对指纹后，执行
CLI 日志或 Web 页面给出的命令：

```bash
ssh-keyscan -p <port> <host> >> ~/.ssh/known_hosts
```

#### 主机密钥已变更

服务器当前提供的主机密钥与 `known_hosts` 中已有记录不一致。在确认变更确实
来自服务器管理员之前，应按潜在中间人攻击处理。确认无误后，删除旧记录（例如
使用 `ssh-keygen -R`），再连接一次写入新密钥。

### SSH config alias 不存在

`Channel '...' references host alias '...', but no Host ... block exists` 表示
`hostname` 拼写有误，或 `~/.ssh/config` 中没有对应的 alias。

### 本地端口已被占用

`Address(es) already in use` 表示配置的 `local` 地址已被其他进程占用。更换端口
或停止该进程即可。可以使用 `lsof -i :PORT`（Linux/macOS）或
`netstat -ano | findstr :PORT`（Windows）查找占用方。

### 特权端口

绑定小于 1024 的端口时，Linux/macOS 需要 root 权限，Windows 需要管理员权限。

### SSH 连接或认证失败

先手动运行 `ssh <alias>`，以区分 SSH config、网络或 key 权限问题。

### 加密私钥

目标 key 被加密时，添加 `[auth.<alias>] passphrase = "..."`。

### Channel 转发失败

从 SSH 服务器验证目标服务可访问，并确认服务器允许 TCP 转发。
`remote->local` channel 需要在服务器端测试。

### 调试日志

`ssh-channels-hub start --debug` 会打印每个 channel 的 SSH 握手、channel 打开和
重连尝试等信息。

具体配置校验错误见 [配置错误参考](./configuration.md#6-配置错误参考)。
