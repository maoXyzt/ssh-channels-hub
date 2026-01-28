# 工作流程文档

## 1. 应用程序启动流程

### 1.1 初始化阶段

```
1. 解析命令行参数
   ├── 命令类型 (start/stop/restart/status/validate)
   ├── 配置文件路径 (可选)
   └── 调试标志 (可选)

2. 初始化日志系统
   ├── 设置日志级别 (根据 --debug 标志)
   ├── 配置日志格式
   └── 初始化 tracing subscriber

3. 加载配置文件
   ├── 确定配置文件路径
   │   ├── 使用 --config 指定的路径
   │   └── 或使用默认路径
   ├── 读取文件内容
   ├── 解析 TOML
   └── 验证配置有效性
```

### 1.2 命令处理流程

#### Start 命令

```
1. 加载配置
   ↓
2. 创建 ServiceManager
   ↓
3. 调用 ServiceManager::start()
   ├── 设置状态为 "Starting"
   ├── 遍历所有 channels 配置
   │   ├── 创建 SshManager
   │   ├── 调用 SshManager::start()
   │   └── 记录启动结果
   ├── 更新状态为 "Running" 或 "Error"
   └── 返回结果
   ↓
4. 如果 foreground 模式
   ├── 等待 Ctrl+C 信号
   └── 调用 ServiceManager::stop()
   ↓
5. 如果 daemon 模式（未来）
   ├── Fork 进程
   ├── 写入 PID 文件
   └── 退出父进程
```

#### Stop 命令

```
1. 读取 PID 文件（未来）
   ↓
2. 发送停止信号（未来）
   ↓
3. 或直接调用 ServiceManager::stop()
   ├── 设置状态为 "Stopping"
   ├── 遍历所有 SshManager
   │   └── 发送关闭信号
   ├── 等待所有任务完成
   └── 设置状态为 "Stopped"
```

#### Restart 命令

```
1. 执行 Stop 命令
   ↓
2. 等待 1 秒
   ↓
3. 执行 Start 命令
```

#### Status 命令

```
1. 读取服务状态（未来：从 PID 文件或 IPC）
   ↓
2. 显示状态信息
   ├── 服务状态
   ├── 活动 channels 数
   └── 总 channels 数
```

#### Validate 命令

```
1. 加载配置文件
   ↓
2. 验证配置
   ├── 检查 TOML 语法
   ├── 检查必需字段
   ├── 检查字段类型
   └── 检查 channel 名称唯一性
   ↓
3. 显示验证结果
```

## 2. SSH 连接建立流程

### 2.1 连接阶段

```
1. SshManager::start() 被调用
   ↓
2. 创建关闭信号通道
   ↓
3. 启动独立任务
   ↓
4. 调用 connect_and_manage_channel()
   ├── 构建重试策略
   └── 使用 backon 重试连接
   ↓
5. establish_connection()
   ├── 创建 SSH 客户端配置
   ├── 创建 ClientHandler
   ├── 连接到服务器
   │   └── russh::client::connect()
   ├── 认证
   │   ├── 密码认证
   │   └── 密钥认证
   └── 打开 channel
       ├── Session channel
       └── Direct-TCPIP channel
```

### 2.2 channel 管理流程

#### Session channel

```
1. channel_open_session()
   ↓
2. 检查是否有命令参数
   ├── 有命令: exec(command)
   └── 无命令: request_pty() + shell()
   ↓
3. 启动 channel 数据处理任务
   ├── 监听 channel 消息
   ├── 处理数据
   └── 检测 channel 关闭
```

#### Direct-TCPIP channel

```
1. channel_open_direct_tcpip()
   ├── 目标地址
   ├── 目标端口
   ├── 源地址（本地）
   └── 源端口（0 = 任意）
   ↓
2. 启动 channel 数据处理任务
   ├── 转发数据
   └── 检测 channel 关闭
```

## 3. 重连流程

### 3.1 连接断开检测

```
连接/channel 错误发生
   ↓
错误被捕获
   ↓
记录错误日志
   ↓
返回到重连逻辑
```

### 3.2 重连执行

```
1. 计算重试延迟
   ├── 指数退避策略
   │   └── delay = min(initial * 2^attempt, max_delay)
   └── 固定间隔策略
       └── delay = initial_delay
   ↓
2. 检查重试限制
   ├── 如果 max_retries > 0
   │   └── 检查是否超过限制
   └── 如果 max_retries == 0
       └── 无限重试
   ↓
3. 等待延迟时间
   ↓
4. 记录重试日志
   ↓
5. 重新建立连接
   └── 回到连接建立流程
```

### 3.3 重连策略示例

**指数退避** (initial=1s, max=30s):

```
尝试 1: 等待 1s
尝试 2: 等待 2s
尝试 3: 等待 4s
尝试 4: 等待 8s
尝试 5: 等待 16s
尝试 6+: 等待 30s (上限)
```

**固定间隔** (delay=5s):

```
尝试 1: 等待 5s
尝试 2: 等待 5s
尝试 3: 等待 5s
...
```

## 4. 关闭流程

### 4.1 正常关闭

```
1. 收到关闭信号 (Ctrl+C 或 stop 命令)
   ↓
2. ServiceManager::stop()
   ├── 设置状态为 "Stopping"
   ├── 遍历所有 SshManager
   │   └── SshManager::stop()
   │       └── 发送关闭信号到任务
   └── 清空管理器列表
   ↓
3. 每个 SshManager 任务
   ├── 收到关闭信号
   ├── 关闭 SSH 连接
   ├── 清理资源
   └── 退出任务
   ↓
4. 设置状态为 "Stopped"
   ↓
5. 退出应用程序
```

### 4.2 异常关闭

```
1. 未捕获的 panic
   ↓
2. 任务终止
   ↓
3. 连接自动关闭
   ↓
4. 其他 channels 继续运行
   ↓
5. 服务状态可能变为 "Error"
```

## 5. 并发执行流程

### 5.1 多 channels 并发

```
主任务
  ├── channel 1 任务 ──┐
  ├── channel 2 任务 ──┤
  ├── channel 3 任务 ──┼──> 独立运行，互不阻塞
  └── channel N 任务 ──┘
```

### 5.2 channel 内部并发

```
SshManager 任务
  ├── 连接管理任务
  ├── channel 数据处理任务 1
  ├── channel 数据处理任务 2
  └── 关闭信号监听任务
```

### 5.3 同步点

- **启动**: 所有 channels 并行启动，不等待其他 channels
- **停止**: 等待所有 channels 完成关闭
- **状态查询**: 需要锁定状态进行读取

## 6. 错误处理流程

### 6.1 错误分类和处理

```
配置错误
  └── 立即失败，不启动服务

连接错误
  ├── 临时性错误 → 重试
  └── 永久性错误 → 记录错误，跳过该 channel

认证错误
  └── 记录错误，跳过该 channel（不重试）

channel 错误
  └── 重试（重新打开 channel）
```

### 6.2 错误传播

```
底层错误 (russh::Error)
   ↓
转换为 AppError
   ↓
添加上下文信息
   ↓
记录日志
   ↓
返回给调用者
   ↓
决定是否重试
```

## 7. 日志记录流程

### 7.1 日志级别使用

- **trace**: 详细的函数调用和状态变化
- **debug**: channel 消息、连接细节
- **info**: 重要事件（连接建立、channel 打开）
- **warn**: 非致命问题（连接关闭、重连）
- **error**: 错误条件（连接失败、认证失败）

### 7.2 结构化日志

```
info!(
    channel = %config.name,    // channel 名称
    host = %config.host,       // host 地址
    port = config.port,        // 端口号
    "Establishing SSH connection"
)
```

### 7.3 日志输出

- **控制台**: 默认输出到 stderr
- **文件**: 未来可能支持日志文件
- **格式**: 由 tracing-subscriber 控制

## 8. 性能优化流程

### 8.1 资源管理

```
连接建立
  ├── 使用 Arc 共享配置（避免复制）
  ├── 及时释放不需要的资源
  └── 使用有界通道（防止内存泄漏）
```

### 8.2 并发优化

```
异步 I/O
  ├── 所有网络操作异步
  ├── 避免阻塞操作
  └── 使用 tokio::spawn_blocking 处理阻塞操作
```

### 8.3 重连优化

```
智能重试
  ├── 指数退避避免资源浪费
  ├── 限制最大重试次数
  └── 可配置的重试策略
```
