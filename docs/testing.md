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

查看运行状态或调试日志：

```bash
ssh-channels-hub status --watch
ssh-channels-hub start --debug
```

测试失败时见 [故障排查](./troubleshooting.md#中文)。
