# Testing Channel Connections / 测试 Channel 连接

> English | [中文](#中文)

## English

### Built-in test

After starting the service, test the local listeners for all `local->remote`
channels:

```bash
ssh-channels-hub test
ssh-channels-hub --config config.toml test
```

When running from source, put `--` before the application arguments:

```bash
cargo run -- test
cargo run -- --config config.toml test
```

`remote->local` channels are skipped. Test them by connecting to their `remote`
addresses from the SSH server.

### Manual verification

Connect to the configured endpoint with a suitable client, for example:

```bash
nc -zv 127.0.0.1 8080
curl http://127.0.0.1:8080
```

### Domain-based remote forwards

For a channel that exposes an internal HTTPS service:

```toml
[[channels]]
name      = "internal-api"
hostname  = "edge-server"
direction = "remote->local"
remote    = "18443"
local     = "https://api.internal.example/v1"
```

First confirm that the daemon machine can access the service. Then check the
remote listener from the SSH server:

```bash
nc -zv 127.0.0.1 18443
```

Test HTTPS while preserving the original hostname for Host, SNI, and certificate
verification:

```bash
curl --connect-to 'api.internal.example:443:127.0.0.1:18443' \
  'https://api.internal.example/v1/models'
```

Replace `/v1/models` with a real endpoint and add any required authentication.
The `--connect-to` option only changes where curl connects; it does not change
the request URL. Do not use `http://` for an HTTPS destination: the daemon
passes TCP through unchanged and does not convert between HTTP and HTTPS.

Check the service status or debug logs:

```bash
ssh-channels-hub status --watch
ssh-channels-hub start --debug
```

If a test fails, see [Troubleshooting](./troubleshooting.md#english).

## 中文

### 内置测试

服务启动后，测试所有 `local->remote` channel 的本地监听端口：

```bash
ssh-channels-hub test
ssh-channels-hub --config config.toml test
```

从源码运行时，在参数前加 `--`：

```bash
cargo run -- test
cargo run -- --config config.toml test
```

`remote->local` channel 会被跳过，需要从 SSH 服务器端连接其 `remote` 地址验证。

### 手动验证

按服务类型连接配置的端点，例如：

```bash
nc -zv 127.0.0.1 8080
curl http://127.0.0.1:8080
```

### 基于域名的反向中转

假设通过以下 channel 暴露内网 HTTPS 服务：

```toml
[[channels]]
name      = "internal-api"
hostname  = "edge-server"
direction = "remote->local"
remote    = "18443"
local     = "https://api.internal.example/v1"
```

先确认运行 daemon 的本机可以访问该服务，然后在 SSH 服务器上检查远端监听：

```bash
nc -zv 127.0.0.1 18443
```

测试 HTTPS 时保留原始域名，以确保 Host、SNI 和证书校验正确：

```bash
curl --connect-to 'api.internal.example:443:127.0.0.1:18443' \
  'https://api.internal.example/v1/models'
```

把 `/v1/models` 换成真实接口，并按需添加认证信息。`--connect-to` 只改变
curl 实际连接的位置，不改变请求 URL。目标是 HTTPS 服务时不要使用
`http://`：daemon 只透传 TCP，不会在 HTTP 和 HTTPS 之间转换协议。

查看运行状态或调试日志：

```bash
ssh-channels-hub status --watch
ssh-channels-hub start --debug
```

测试失败时见 [故障排查](./troubleshooting.md#中文)。
