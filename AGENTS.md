# AGENTS.md

You are an expert in Rust, async programming, and concurrent systems.

## Project Tech Stack

**Required Libraries:**

- **`russh`** - SSH connections and SSH channel management
- **`clap`** - Command-line argument parsing and CLI interface
- **`anyhow`** - Application-level error handling with context (`Result<T, anyhow::Error>`)
- **`thiserror`** - Custom error types with automatic `Error` trait implementation
- **`tracing`** - Structured logging macros (`info!`, `debug!`, `error!`, `warn!`, `trace!`)
- **`tracing-subscriber`** - Log output formatting and filtering
- **`tokio`** - Async runtime for tasks and I/O

**Configuration:**

- Prefer **TOML** format for configuration files
- Use environment variables for configuration management (e.g., `dotenv` crate)

## Code Conventions

### Naming and Style

- Use expressive variable names that convey intent (e.g., `is_ready`, `has_data`)
- Follow Rust naming conventions:
  - `snake_case` for variables and functions
  - `PascalCase` for types and structs
  - `SCREAMING_SNAKE_CASE` for constants

### Code Organization

- Structure the application into modules: separate concerns like networking, database, and business logic
- Avoid code duplication; use functions and modules to encapsulate reusable logic
- Ensure code is well-documented with inline comments and Rustdoc

### General Principles

- Write clear, concise, and idiomatic Rust code
- Prioritize modularity, clean code organization, and efficient resource management
- Write code with safety, concurrency, and performance in mind, embracing Rust's ownership and type system

## Async Programming Patterns

### Basic Async

- Use `tokio` as the async runtime for handling asynchronous tasks and I/O
- Implement async functions using `async fn` syntax
- Use `.await` responsibly, ensuring safe points for context switching
- Use `?` operator to propagate errors in async functions

### Task Management

- Leverage `tokio::spawn` for task spawning and concurrency
- Use `tokio::select!` for managing multiple async tasks and cancellations
- Favor structured concurrency: prefer scoped tasks and clean cancellation paths
- Use `tokio::task::yield_now` to yield control in cooperative multitasking scenarios

### Time Operations

- Use `tokio::time::sleep` and `tokio::time::interval` for efficient time-based operations
- Implement timeouts, retries, and backoff strategies for robust async operations

### Blocking Operations

- Avoid blocking operations inside async functions
- Offload blocking operations to dedicated blocking threads if necessary (use `tokio::task::spawn_blocking`)

## Concurrency & Channels

### Channel Types

- **`tokio::sync::mpsc`** - Asynchronous, multi-producer, single-consumer channels
- **`tokio::sync::broadcast`** - Broadcasting messages to multiple consumers
- **`tokio::sync::oneshot`** - One-time communication between tasks

### Channel Best Practices

- Prefer bounded channels for backpressure; handle capacity limits gracefully
- Use unbounded channels only when backpressure is not a concern

### Shared State

- Use `tokio::sync::Mutex` for shared mutable state across tasks
- Use `tokio::sync::RwLock` when multiple readers are needed
- Avoid deadlocks by acquiring locks in a consistent order
- Minimize lock duration to reduce contention

## Error Handling

### Error Types

- Embrace Rust's `Result<T, E>` and `Option<T>` types for error handling
- Use `anyhow::Result<T>` for application-level error handling with context
- Use `thiserror` to define custom error types with automatic `Error` trait implementation

### Error Propagation

- Use `?` operator to propagate errors in async functions
- Add context to errors using `anyhow::Context::context()` or `.with_context()`
- Handle errors and edge cases early, returning errors where appropriate

### Error Examples

```rust
// Using anyhow for application errors
use anyhow::{Context, Result};

async fn connect() -> Result<()> {
    let config = load_config()
        .context("Failed to load configuration")?;
    // ...
}

// Using thiserror for custom error types
use thiserror::Error;

#[derive(Error, Debug)]
enum SshError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Authentication failed")]
    AuthenticationFailed,
}
```

## Logging

### Logging Macros

- Use `tracing::info!()` for informational messages
- Use `tracing::debug!()` for debugging information
- Use `tracing::error!()` for error conditions
- Use `tracing::warn!()` for warnings
- Use `tracing::trace!()` for very detailed tracing

### Logging Setup

- Configure `tracing-subscriber` for log output formatting and filtering
- Use structured logging with fields: `info!(field = value, "message")`
- Set appropriate log levels based on environment (debug in development, info in production)

## Testing

### Async Tests

- Write unit tests with `#[tokio::test]` for async tests
- Use `tokio::time::pause()` for testing time-dependent code without real delays

### Test Organization

- Implement integration tests to validate async behavior and concurrency
- Use mocks and fakes for external dependencies in tests
- Test error paths and edge cases, not just happy paths

## Performance Optimization

### Async Efficiency

- Minimize async overhead; use sync code where async is not needed
- Optimize data structures and algorithms for async use, reducing contention and lock duration

### Resource Management

- Use efficient data structures that minimize allocations
- Consider using `Arc` for shared ownership when needed
- Use `tokio::sync::Mutex` instead of `std::sync::Mutex` in async contexts

## Quick Reference

### Common Patterns

**SSH Connection with russh:**

```rust
use russh::*;
// Use russh client/server APIs for SSH connections
```

**CLI with clap:**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
```

**Error Handling:**

```rust
use anyhow::{Context, Result};

fn function() -> Result<()> {
    let value = risky_operation()
        .context("Failed to perform operation")?;
    Ok(())
}
```

**Structured Logging:**

```rust
use tracing::{info, debug, error};

info!(user_id = 123, "User logged in");
debug!(connection_id = "abc", "Establishing connection");
error!(error = ?err, "Operation failed");
```

**Async Task Spawning:**

```rust
let handle = tokio::spawn(async {
    // async work
});
let result = handle.await?;
```

**Channel Communication:**

```rust
let (tx, mut rx) = tokio::sync::mpsc::channel(100);
tokio::spawn(async move {
    tx.send(value).await?;
});
let received = rx.recv().await;
```

---

Refer to Rust's async book and `tokio` documentation for in-depth information on async patterns, best practices, and advanced features.
