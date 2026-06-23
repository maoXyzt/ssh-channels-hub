# 模块设计文档

## 1. 模块概览

项目采用模块化设计，每个模块负责特定的功能领域。

```
src/
├── main.rs      # 程序入口，CLI 处理
├── cli.rs       # 命令行接口定义
├── config.rs    # 配置加载和解析
├── error.rs     # 错误类型定义
├── service.rs   # 服务管理
└── ssh.rs       # SSH 连接和 channel 管理
```

## 2. 模块详细说明

### 2.1 main.rs

**职责**: 程序入口点，协调各个模块

**主要功能**:

- 初始化日志系统
- 解析 CLI 参数
- 路由命令到对应的处理函数
- 管理应用程序生命周期

**关键函数**:

- `main()`: 异步主函数
- `init_logging()`: 初始化 tracing 日志系统
- `handle_start()`: 处理启动命令
- `handle_stop()`: 处理停止命令
- `handle_restart()`: 处理重启命令
- `handle_status()`: 处理状态查询命令
- `handle_validate()`: 处理配置验证命令

### 2.2 cli.rs

**职责**: 定义命令行接口

**数据结构**:

```rust
pub struct Cli {
    pub command: Commands,
    pub config: Option<PathBuf>,
    pub debug: bool,
}

pub enum Commands {
    Start { foreground: bool },
    Stop,
    Restart,
    Status,
    Validate { config: Option<PathBuf> },
}
```

**设计特点**:

- 使用 `clap` 的 derive 宏自动生成 CLI
- 支持全局选项（`--config`, `--debug`）
- 子命令模式，清晰的命令结构

### 2.3 config.rs

**职责**: 加载 `config.toml`、解析、并联合 `~/.ssh/config` 构造运行时 ChannelConfig。

host info(HostName / User / Port / IdentityFile / ProxyJump)由 `ssh_config.rs` 从 `~/.ssh/config` 读出,`config.rs` 只负责 channels、auth 覆盖、重连策略,以及把 `ProxyJump` 链解析成跳板配置。

**核心数据结构**:

```rust
pub struct AppConfig {
    pub ssh_config: Option<PathBuf>,           // 覆盖 ~/.ssh/config 路径(可选)
    pub channels: Vec<ConnectionConfig>,
    pub auth: HashMap<String, AuthOverride>,   // 键为 SSH config 的 Host alias
    pub reconnection: ReconnectionConfig,
}

pub struct ConnectionConfig {
    pub name: String,
    pub hostname: String,                      // SSH config 里的 Host alias
    pub direction: Direction,                  // "local->remote" 或 "remote->local"
    pub local: Endpoint,                       // 本机这一侧的 host:port
    pub remote: Endpoint,                      // 远端这一侧的 host:port
}

pub enum Direction {
    LocalToRemote,                             // ssh -L: 本机监听,流量出
    RemoteToLocal,                             // ssh -R: 服务器绑定,流量入
}

pub struct Endpoint {
    pub host: String,                          // 默认 "127.0.0.1"
    pub port: u16,
}

pub struct AuthOverride {
    pub password: Option<String>,              // SSH config 存不了的密码
    pub passphrase: Option<String>,            // IdentityFile 的 passphrase
}

// Runtime channel configuration (built from config.toml + ~/.ssh/config)
pub struct ChannelConfig {
    pub name: String,
    pub host: String,                          // resolved from SSH HostName
    pub port: u16,                             // resolved from SSH Port, default 22
    pub username: String,                      // resolved from SSH User
    pub auth: AuthConfig,
    pub params: ChannelTypeParams,             // DirectTcpIp / ForwardedTcpIp
    pub proxy_jumps: Vec<JumpHopConfig>,       // 解析后的 ProxyJump 链(顺序敏感,空 = 直连)
}

pub enum AuthConfig {
    Password { password: String },
    Key { key_path: PathBuf, passphrase: Option<String> },
}

// 单个跳板的运行时配置。仅来源于 ~/.ssh/config 的 Host 别名 + IdentityFile —
// `config.toml` 不允许为跳板单独配 auth(跳板只支持 publickey)。
pub struct JumpHopConfig {
    pub alias: String,                         // ssh_config 中的 Host alias
    pub host: String,                          // HostName
    pub port: u16,                             // Port,默认 22
    pub username: String,                      // User
    pub key_path: PathBuf,                     // 显式 IdentityFile > Host * > 唯一默认 key
}
```

**主要功能**:

- `AppConfig::from_file()`: 加载并反序列化
- `AppConfig::default_path()`: 获取默认 config.toml 路径
- `AppConfig::ssh_config_path()`: 计算 SSH config 实际路径(`ssh_config` 字段优先,否则 `~/.ssh/config`)
- `AppConfig::build_channels()`: 解析 SSH config、按 alias 查表、套用 auth 覆盖、解析 `ProxyJump` 链(`resolve_jump_chain`),构造 `Vec<ChannelConfig>`
- `AppConfig::generate_scaffold()`: 从 SSH config 条目渲染一份注释掉的 `config.toml` 文本,供 `generate` 子命令使用
- `check_jump_preflight()`: 跳板环境前置检查 —— IdentityFile 文件是否存在(error)、跳板主机是否在 `~/.ssh/known_hosts`(warning);供 `validate` 命令调用

**auth 解析规则(`resolve_auth`)**:

1. `[auth.<alias>].password` 存在 → 走密码登录,**覆盖** SSH config 的 IdentityFile
2. 否则 SSH config 的 `IdentityFile` 存在 → 走密钥登录,`[auth.<alias>].passphrase` 附加到密钥上
3. 否则报错,提示「填一个 password 或者补 IdentityFile」

**设计考虑**:

- 单一信息源:host info 完全来自 SSH config,避免与 `~/.ssh/config` 重复维护
- 配置失败提前暴露:`build_channels` 在 service 启动前调用,任何不一致一次性报出

### 2.4 error.rs

**职责**: 定义应用程序错误类型

**错误类型**:

```rust
pub enum AppError {
    Config(String),
    SshConnection(String),
    SshAuthentication(String),
    SshChannel(String),
    Io(std::io::Error),
    ConfigParse(toml::de::Error),
    Service(String),
}
```

**设计特点**:

- 使用 `thiserror` 自动实现 `Error` trait
- 支持错误链（通过 `#[from]` 属性）
- 提供上下文信息
- 类型别名 `Result<T>` 简化错误处理

### 2.5 service.rs

**职责**: 管理所有 SSH channels 的服务生命周期

**核心数据结构**:

```rust
pub struct ServiceManager {
    config: AppConfig,
    state: Arc<Mutex<ServiceState>>,
    managers: Arc<Mutex<Vec<SshManager>>>,
}

pub enum ServiceState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error(String),
}

// 每条 channel 的实时健康度,由 SshManager 的 connect 任务写入,
// 由 ServiceManager::status() 读出供 CLI 渲染。
pub enum ChannelHealth {
    Stopped,
    Connecting { attempt: u32 },
    Connected,
    Reconnecting { attempt: u32, last_error: String },
    Failed { error: String },
}

pub struct ChannelStatus {
    pub name: String,
    pub direction: Direction,
    pub local: String,           // host:port
    pub remote: String,
    pub health: ChannelHealth,
}

pub struct ServiceStatus {
    pub state: ServiceState,
    pub channels: Vec<ChannelStatus>,
}
```

**主要功能**:

- `start()`: 启动所有 channels
- `stop()`: 停止所有 channels
- `restart()`: 重启服务
- `status()`: 遍历每个 `SshManager.snapshot()`,返回包含每条 channel 实时健康度的 `ServiceStatus`(供 `status` 命令和 `--watch` 模式渲染)

**设计特点**:

- 使用 `Arc<Mutex<>>` 管理共享状态
- 状态机模式管理服务状态
- 优雅处理部分 channels 启动失败的情况
- 提供详细的状态信息

**并发安全**:

- 所有状态访问都通过 `Mutex` 保护
- 异步操作使用 `tokio::sync::Mutex`
- 避免死锁的设计模式

### 2.6 ssh.rs

**职责**: 管理单个 SSH 连接和 channel

**核心数据结构**:

```rust
pub struct SshManager {
    config: ChannelConfig,
    reconnection_config: ReconnectionConfig,
    shutdown_tx: Option<mpsc::Sender<()>>,
    cancellation_token: Option<CancellationToken>,
    // 实时健康度,与 spawn 出去的 connect 任务共享。
    // 使用 std::sync::Mutex(不跨 await),写者:spawn 任务的状态转移
    // 与 backon 的 .notify 钩子;读者:SshManager::snapshot()。
    health: Arc<std::sync::Mutex<ChannelHealth>>,
}

struct ClientHandler;              // 终点 SSH 会话
struct ReverseForwardHandler { … } // ssh -R 服务端推回的 forwarded-tcpip
struct JumpClientHandler { … }     // ProxyJump 跳板专用,严格校验 known_hosts
```

**主要功能**:

1. **连接管理**:
   - `establish_connection()`: 走完跳板链 + 终点认证 + 启动 channel-side 服务(listener bind / tcpip-forward)
   - `connect_via_chain()`: 按顺序把 `ProxyJump` 链全部认证起来,跳板 handle 由调用者持有以保证生命周期
   - `load_secret_key()` / `load_jump_key()`: 加载私钥(后者强制拒绝 passphrase 加密的 key)

2. **重连逻辑**:
   - `connect_and_manage_channel()`: 用 `backon` 做指数退避重试,`.notify` 钩子把状态切到 `Reconnecting{ attempt, last_error }`,外层循环负责 max_retries 用完后的 1 秒重置

3. **生命周期管理**:
   - `start()`: spawn 出连接任务,初始化 `health = Connecting{1}`
   - `stop()`: shutdown + cancel,把 `health` 置 `Stopped`
   - `snapshot()`: 同步返回当前 channel 的 `ChannelStatus`(名字 / 方向 / 端点 / 健康度),供 `ServiceManager::status` 调用

4. **状态写入点**(谁把 ChannelHealth 设成谁):
   - 循环入口 → `Connecting{1}`
   - backon `.notify` 钩子 → `Reconnecting{n, err}`
   - `run_direct_tcpip_listener` 在 `TcpListener::bind` 成功后 → `Connected`
   - `drive_forwarded_tcpip` 在 `tcpip_forward` 返回 Ok 后 → `Connected`
   - 单次 retry cycle 用尽 → `Failed{err}`(外层 1s 后重置为 `Connecting{1}`)
   - shutdown / cancel → `Stopped`

**设计特点**:

- 每个管理器运行在独立任务中
- 使用 `tokio::select!` 处理关闭信号
- 自动重连机制
- 支持多种 channel 类型

**重连策略**:

- 指数退避（默认）
- 固定间隔（可选）
- 可配置最大重试次数
- 可配置延迟范围

## 3. 模块间依赖关系

```
main.rs
  ├── cli.rs (CLI 定义)
  ├── config.rs (配置加载)
  ├── service.rs (服务管理)
  │     └── ssh.rs (SSH 连接)
  │           └── config.rs (配置结构)
  └── error.rs (错误类型)
```

## 4. 模块接口设计原则

### 4.1 单一职责原则

每个模块只负责一个明确的功能领域。

### 4.2 最小接口原则

模块只暴露必要的公共 API，内部实现细节隐藏。

### 4.3 错误处理一致性

所有模块使用统一的错误类型 (`AppError`)，通过 `Result<T>` 类型别名简化。

### 4.4 异步优先

所有 I/O 操作都是异步的，使用 `async/await` 语法。

## 5. 扩展点

### 5.1 添加新的 channel 类型

在 `ssh.rs` 中添加新的 channel 打开函数，在 `establish_connection()` 中添加分支。

### 5.2 添加新的认证方式

在 `config.rs` 的 `AuthConfig` 中添加新变体，在 `ssh.rs` 的认证逻辑中添加处理。

### 5.3 自定义重连策略

在 `config.rs` 中添加配置选项，在 `ssh.rs` 中实现策略逻辑。

### 5.4 添加监控和指标

可以在 `service.rs` 中添加指标收集，或创建新的 `metrics.rs` 模块。

## 6. 测试策略

### 6.1 单元测试

- 配置解析测试 (`config.rs`)
- 错误处理测试 (`error.rs`)
- 状态管理测试 (`service.rs`)

### 6.2 集成测试

- SSH 连接测试（需要测试服务器）
- 重连逻辑测试
- CLI 命令测试

### 6.3 模拟测试

- 使用 mock SSH 服务器测试连接逻辑
- 模拟网络故障测试重连
