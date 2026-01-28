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
└── ssh.rs       # SSH 连接和通道管理
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

**职责**: 配置文件的加载、解析和验证

**核心数据结构**:

```rust
pub struct AppConfig {
    pub channels: Vec<ChannelConfig>,
    pub reconnection: ReconnectionConfig,
}

pub struct ChannelConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthConfig,
    pub channel_type: String,
    pub params: ChannelParams,
}

pub enum AuthConfig {
    Password { password: String },
    Key { key_path: PathBuf, passphrase: Option<String> },
    Agent,
}
```

**主要功能**:

- `AppConfig::from_file()`: 从文件加载配置
- `AppConfig::default_path()`: 获取默认配置路径
- 使用 `serde` 进行 TOML 反序列化
- 提供默认值支持

**设计考虑**:

- 使用枚举类型确保类型安全
- 支持可选字段和默认值
- 清晰的错误信息

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

**职责**: 管理所有 SSH 通道的服务生命周期

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
```

**主要功能**:

- `start()`: 启动所有通道
- `stop()`: 停止所有通道
- `restart()`: 重启服务
- `status()`: 获取服务状态

**设计特点**:

- 使用 `Arc<Mutex<>>` 管理共享状态
- 状态机模式管理服务状态
- 优雅处理部分通道启动失败的情况
- 提供详细的状态信息

**并发安全**:

- 所有状态访问都通过 `Mutex` 保护
- 异步操作使用 `tokio::sync::Mutex`
- 避免死锁的设计模式

### 2.6 ssh.rs

**职责**: 管理单个 SSH 连接和通道

**核心数据结构**:

```rust
pub struct SshManager {
    config: ChannelConfig,
    reconnection_config: ReconnectionConfig,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

struct ClientHandler;
```

**主要功能**:

1. **连接管理**:
   - `establish_connection()`: 建立 SSH 连接
   - `connect_ssh_session()`: 连接到 SSH 服务器
   - `load_secret_key()`: 加载私钥文件

2. **通道管理**:
   - `open_session_channel()`: 打开会话通道
   - `open_direct_tcpip_channel()`: 打开端口转发通道

3. **重连逻辑**:
   - `connect_and_manage_channel()`: 带重试的连接管理
   - 使用 `backon` 实现重试策略

4. **生命周期管理**:
   - `start()`: 启动管理器
   - `stop()`: 停止管理器

**设计特点**:

- 每个管理器运行在独立任务中
- 使用 `tokio::select!` 处理关闭信号
- 自动重连机制
- 支持多种通道类型

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

### 5.1 添加新的通道类型

在 `ssh.rs` 中添加新的通道打开函数，在 `establish_connection()` 中添加分支。

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
