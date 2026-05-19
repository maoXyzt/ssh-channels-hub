mod cli;
mod config;
mod error;
mod port_check;
mod service;
mod ssh;
mod ssh_config;
mod ui;

use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
use clap::Parser;
use cli::{Cli, Commands};
use config::AppConfig;
use port_check::{test_port_connection, test_tunnel_connection};
use service::{ServiceManager, ServiceState};
use ssh_config::{default_ssh_config_path, parse_ssh_config};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> AnyhowResult<()> {
  let cli = Cli::parse();

  // Honor --no-color (NO_COLOR env var is already respected by owo-colors).
  if cli.no_color {
    ui::disable_colors();
  }

  // Initialize logging
  init_logging(cli.debug)?;

  // Determine config path
  let config_path = cli.config.clone().unwrap_or_else(AppConfig::default_path);

  // Handle commands
  match cli.command {
    Commands::Start { daemon } => {
      handle_start(config_path, daemon, cli.debug).await?;
    }
    Commands::Stop => {
      handle_stop(config_path).await?;
    }
    Commands::Restart => {
      handle_restart(config_path, cli.debug).await?;
    }
    Commands::Status => {
      handle_status(config_path).await?;
    }
    Commands::Validate { config } => {
      let path = config.or(Some(config_path));
      handle_validate(path).await?;
    }
    Commands::Generate { ssh_config, output } => {
      handle_generate(ssh_config, output).await?;
    }
    Commands::Test { config } => {
      let test_config_path = config.unwrap_or_else(AppConfig::default_path);
      handle_test(test_config_path).await?;
    }
  }

  Ok(())
}

/// Spawn a detached child process that runs the service (foreground mode). Parent exits immediately.
async fn spawn_daemon(config_path: &Path, debug: bool) -> AnyhowResult<()> {
  let exe = std::env::current_exe().context("Get current executable")?;
  let mut cmd = Command::new(&exe);
  cmd
    .arg("start")
    .arg("--config")
    .arg(config_path)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
  if debug {
    cmd.arg("--debug");
  }

  #[cfg(windows)]
  {
    // DETACHED_PROCESS = 8: child has no console and survives parent exit
    const DETACHED_PROCESS: u32 = 0x00000008;
    cmd.creation_flags(DETACHED_PROCESS);
  }

  cmd.spawn().context("Spawn daemon process")?;

  tokio::time::sleep(Duration::from_millis(800)).await;
  ui::success("Service started in daemon mode");
  ui::hint("Run `ssh-channels-hub status` to inspect the live state.");
  Ok(())
}

/// Initialize logging subsystem
fn init_logging(debug: bool) -> AnyhowResult<()> {
  let filter = if debug {
    EnvFilter::new("debug")
  } else {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
  };

  tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_target(false)
    .init();

  Ok(())
}

/// Handle start command
async fn handle_start(
  config_path: std::path::PathBuf,
  daemon: bool,
  debug: bool,
) -> AnyhowResult<()> {
  if daemon {
    spawn_daemon(&config_path, debug).await?;
    return Ok(());
  }

  ui::header("🚀", "Starting ssh-channels-hub");
  ui::kv_dim("Config", config_path.display());

  let config = AppConfig::from_file(&config_path).context("Failed to load configuration")?;
  ui::kv("Channels", config.channels.len());
  ui::kv_dim("SSH config", config.ssh_config_path().display());

  info!("Configuration loaded successfully");

  let service_manager = Arc::new(ServiceManager::new(config));

  // Start the service
  service_manager
    .start()
    .await
    .context("Failed to start service")?;

  // Start IPC listener so "status" command can query this process
  let cancel = CancellationToken::new();
  let port = start_ipc_listener(&config_path, Arc::clone(&service_manager), cancel.clone())
    .await
    .context("Failed to start IPC listener for status queries")?;
  write_pid_file(&pid_file_path(&config_path)).context("Write PID file")?;
  debug!(
    "Status query listener on 127.0.0.1:{} (status command will connect here)",
    port
  );

  println!();
  ui::info("Service running in foreground. Press Ctrl+C to stop.");

  tokio::select! {
      _ = tokio::signal::ctrl_c() => {}
      _ = cancel.cancelled() => {}
  }

  println!();
  ui::step("Shutdown signal received, stopping service...");

  cancel.cancel();
  let _ = remove_run_files(&config_path);
  service_manager
    .stop()
    .await
    .context("Failed to stop service")?;

  ui::success("Service stopped cleanly.");
  Ok(())
}

// ----- IPC: status command connects to main process -----

fn run_dir(config_path: &Path) -> PathBuf {
  config_path
    .parent()
    .unwrap_or_else(|| Path::new("."))
    .to_path_buf()
}

fn pid_file_path(config_path: &Path) -> PathBuf {
  run_dir(config_path).join("ssh-channels-hub.pid")
}

fn port_file_path(config_path: &Path) -> PathBuf {
  run_dir(config_path).join("ssh-channels-hub.port")
}

/// Write PID file (plain text, one number) - standard for Linux daemons.
fn write_pid_file(path: &Path) -> AnyhowResult<()> {
  let pid = std::process::id();
  std::fs::write(path, pid.to_string()).context("Write PID file")?;
  Ok(())
}

/// Write port file (plain text, one number) so status command knows where to connect.
fn write_port_file(path: &Path, port: u16) -> AnyhowResult<()> {
  std::fs::write(path, port.to_string()).context("Write port file")?;
  Ok(())
}

fn remove_run_files(config_path: &Path) -> AnyhowResult<()> {
  for path in [pid_file_path(config_path), port_file_path(config_path)] {
    if path.exists() {
      let _ = std::fs::remove_file(&path);
    }
  }
  Ok(())
}

/// Serialize ServiceStatus to TOML (one-way protocol: server sends, client reads).
fn status_to_toml(status: &service::ServiceStatus) -> String {
  let state_str = match &status.state {
    ServiceState::Running => "Running",
    ServiceState::Stopped => "Stopped",
    ServiceState::Starting => "Starting",
    ServiceState::Stopping => "Stopping",
    ServiceState::Error(_) => "Error",
  };
  format!(
    "state = \"{}\"\nactive_channels = {}\ntotal_channels = {}",
    state_str, status.active_channels, status.total_channels
  )
}

/// Bind TCP on 127.0.0.1:0, write port to file, spawn task that accepts connections and responds with current status.
async fn start_ipc_listener(
  config_path: &Path,
  service_manager: Arc<ServiceManager>,
  cancel: CancellationToken,
) -> AnyhowResult<u16> {
  let listener = TcpListener::bind("127.0.0.1:0")
    .await
    .context("Bind IPC listener")?;
  let port = listener
    .local_addr()
    .context("Get IPC listener port")?
    .port();
  write_port_file(&port_file_path(config_path), port)?;

  let config_path = config_path.to_path_buf();

  tokio::spawn(async move {
    loop {
      tokio::select! {
          _ = cancel.cancelled() => {
              debug!("IPC listener cancelled");
              break;
          }
          accept_result = listener.accept() => {
              match accept_result {
                  Ok((stream, _addr)) => {
                      let manager = Arc::clone(&service_manager);
                      let shutdown = cancel.clone();
                      tokio::spawn(async move {
                          if let Err(e) = handle_ipc_connection(stream, manager, shutdown).await {
                              debug!(error = ?e, "IPC connection handler error");
                          }
                      });
                  }
                  Err(e) => {
                      if !cancel.is_cancelled() {
                          debug!(error = ?e, "IPC accept error");
                      }
                      break;
                  }
              }
          }
      }
    }
    let _ = remove_run_files(&config_path);
  });

  Ok(port)
}

/// Read one line (until \n) from stream.
async fn read_line_async(stream: &mut TcpStream) -> AnyhowResult<String> {
  let mut buf = Vec::new();
  let mut one = [0u8; 1];
  loop {
    let n = stream.read(&mut one).await?;
    if n == 0 {
      break;
    }
    if one[0] == b'\n' {
      break;
    }
    buf.push(one[0]);
  }
  Ok(String::from_utf8(buf).unwrap_or_default())
}

/// Handle one IPC connection: read command line ("status" or "stop"). "stop" -> cancel shutdown and reply "ok"; else -> reply status TOML.
async fn handle_ipc_connection(
  mut stream: TcpStream,
  service_manager: Arc<ServiceManager>,
  shutdown: CancellationToken,
) -> AnyhowResult<()> {
  let cmd = read_line_async(&mut stream).await?.trim().to_lowercase();
  if cmd == "stop" {
    shutdown.cancel();
    stream.write_all(b"ok\n").await?;
    stream.shutdown().await?;
    return Ok(());
  }
  let status = service_manager.status().await;
  let body = status_to_toml(&status);
  stream.write_all(body.as_bytes()).await?;
  stream.shutdown().await?;
  Ok(())
}

/// Read port file (plain text) and connect to main process to fetch status.
async fn query_status_via_ipc(config_path: &Path) -> AnyhowResult<service::ServiceStatus> {
  let port_path = port_file_path(config_path);
  let content =
    std::fs::read_to_string(&port_path).context("Read port file (is service running?)")?;
  let port: u16 = content.trim().parse().context("Parse port file")?;
  let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
    .await
    .context("Connect to service (is it running?)")?;
  stream.write_all(b"status\n").await?;
  stream.shutdown().await?;
  let mut buf = Vec::new();
  stream.read_to_end(&mut buf).await?;
  let body = String::from_utf8(buf).context("IPC response not UTF-8")?;
  parse_status_toml(&body).context("Parse status response")
}

#[derive(serde::Deserialize)]
struct StatusResponse {
  state: String,
  active_channels: usize,
  total_channels: usize,
}

fn parse_status_toml(s: &str) -> AnyhowResult<service::ServiceStatus> {
  let r: StatusResponse = toml::from_str(s).context("Parse status TOML")?;
  let state = match r.state.as_str() {
    "Running" => ServiceState::Running,
    "Stopped" => ServiceState::Stopped,
    "Starting" => ServiceState::Starting,
    "Stopping" => ServiceState::Stopping,
    "Error" => ServiceState::Error(String::new()),
    _ => return Err(anyhow::anyhow!("Unknown state: {}", r.state)),
  };
  Ok(service::ServiceStatus {
    state,
    active_channels: r.active_channels,
    total_channels: r.total_channels,
  })
}

/// Send "stop" via IPC so daemon exits gracefully; then remove run files.
async fn send_stop_via_ipc(config_path: &Path) -> AnyhowResult<()> {
  let port_path = port_file_path(config_path);
  let content =
    std::fs::read_to_string(&port_path).context("Read port file (is service running?)")?;
  let port: u16 = content.trim().parse().context("Parse port file")?;
  let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
    .await
    .context("Connect to service (is it running?)")?;
  stream.write_all(b"stop\n").await?;
  stream.shutdown().await?;
  let mut buf = vec![0u8; 8];
  let _ = stream.read(&mut buf).await;
  Ok(())
}

/// Handle stop command: send "stop" via IPC so daemon exits, then remove run files.
async fn handle_stop(config_path: PathBuf) -> AnyhowResult<()> {
  ui::header("🛑", "Stopping ssh-channels-hub");

  let mut signalled = false;
  if port_file_path(&config_path).exists() {
    match send_stop_via_ipc(&config_path).await {
      Ok(()) => {
        ui::step("Sent stop signal to the running service.");
        signalled = true;
        tokio::time::sleep(Duration::from_millis(600)).await;
      }
      Err(e) => {
        ui::warn(format!("Could not reach service via IPC: {}", e));
      }
    }
  } else {
    ui::info("No PID/port file found — service may not be running.");
  }

  remove_run_files(&config_path).context("Remove run files")?;
  if signalled {
    ui::success("Service stopped.");
  } else {
    ui::success("Run files cleaned up.");
  }
  Ok(())
}

/// Handle restart command: stop running service via IPC (if any), then start as daemon.
async fn handle_restart(config_path: std::path::PathBuf, debug: bool) -> AnyhowResult<()> {
  ui::header("🔄", "Restarting ssh-channels-hub");

  if port_file_path(&config_path).exists() {
    match send_stop_via_ipc(&config_path).await {
      Ok(()) => {
        ui::step("Stopping the running service…");
        tokio::time::sleep(Duration::from_millis(700)).await;
      }
      Err(e) => {
        debug!("No running service or IPC failed: {}", e);
        ui::info("No running service detected, starting fresh.");
      }
    }
    let _ = remove_run_files(&config_path);
  } else {
    ui::info("No running service detected, starting fresh.");
  }

  ui::step("Spawning service in daemon mode…");
  spawn_daemon(&config_path, debug)
    .await
    .context("Failed to start service after restart")?;
  ui::success("Service restarted.");
  Ok(())
}

/// Handle status command: connect to main process via IPC to get live status.
async fn handle_status(config_path: PathBuf) -> AnyhowResult<()> {
  // Try IPC first: connect to running main process
  if let Ok(status) = query_status_via_ipc(&config_path).await {
    let pid = std::fs::read_to_string(pid_file_path(&config_path)).ok();
    let pid_str = pid
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .map(|s| s.to_string());

    ui::print_service_status(&status, &config_path, pid_str.as_deref(), None);

    if let Ok(config) = AppConfig::from_file(&config_path) {
      println!();
      ui::channel_list(&config.channels);
    }
    return Ok(());
  }

  // No running process (IPC file missing or connection refused): show Stopped with config totals
  if !config_path.exists() {
    ui::header("📋", "Service Status");
    ui::fail(format!(
      "Configuration file not found: {}",
      config_path.display()
    ));
    ui::hint("Run `ssh-channels-hub generate` to scaffold a config.toml from ~/.ssh/config.");
    return Ok(());
  }

  match AppConfig::from_file(&config_path) {
    Ok(config) => {
      let total = config.channels.len();
      let status = service::ServiceStatus {
        state: ServiceState::Stopped,
        active_channels: 0,
        total_channels: total,
      };
      ui::print_service_status(
        &status,
        &config_path,
        None,
        Some("Service is not running. Start with: `ssh-channels-hub start -D`"),
      );
      println!();
      ui::channel_list(&config.channels);
    }
    Err(e) => {
      ui::header("📋", "Service Status");
      ui::fail(format!("Failed to load configuration: {}", e));
      return Err(anyhow::anyhow!("Failed to load config: {}", e));
    }
  }

  Ok(())
}

/// Handle validate command
async fn handle_validate(config_path: Option<std::path::PathBuf>) -> AnyhowResult<()> {
  let path = config_path
    .ok_or_else(|| anyhow::anyhow!("Configuration file path required for validation"))?;

  ui::header("🔍", "Validating configuration");
  ui::kv_dim("Config", path.display());

  let config = match AppConfig::from_file(&path) {
    Ok(c) => c,
    Err(e) => {
      println!();
      ui::fail(format!("Configuration could not be parsed: {}", e));
      return Err(anyhow::anyhow!("Invalid configuration: {}", e));
    }
  };

  ui::kv_dim("SSH config", config.ssh_config_path().display());

  // Resolve channels against ~/.ssh/config — this is what actually catches missing
  // host aliases, missing User/HostName/IdentityFile, and bad port specs.
  let channels = match config.build_channels() {
    Ok(c) => c,
    Err(e) => {
      println!();
      ui::fail(format!("Channels failed to resolve: {}", e));
      ui::hint(
        "Check that each `hostname` matches a `Host` alias in ~/.ssh/config and has HostName/User set.",
      );
      return Err(anyhow::anyhow!("Invalid configuration: {}", e));
    }
  };

  // ProxyJump environment checks (IdentityFile on disk, known_hosts entries).
  // Errors fail validation; warnings are printed but pass.
  let preflight = config::check_jump_preflight(&channels, None);

  if !preflight.warnings.is_empty() {
    println!();
    for w in &preflight.warnings {
      ui::warn(w);
    }
  }

  if !preflight.errors.is_empty() {
    println!();
    for e in &preflight.errors {
      ui::fail(e);
    }
    ui::hint(
      "Fix the listed IdentityFile paths in ~/.ssh/config (or place the key file at the resolved path).",
    );
    return Err(anyhow::anyhow!(
      "ProxyJump preflight failed: {} error(s)",
      preflight.errors.len()
    ));
  }

  println!();
  ui::success(format!(
    "Configuration is valid — {} channel(s) resolved.",
    channels.len()
  ));

  if !channels.is_empty() {
    println!();
    ui::subheader("  Resolved channels:");
    for ch in &channels {
      ui::resolved_channel_line(&ch.name, &ch.username, &ch.host, ch.port, &ch.params);
    }
  }
  Ok(())
}

/// Handle generate command: scaffold a config.toml from existing SSH config aliases.
///
/// Emits one commented-out `[[channels]]` template per SSH alias plus a
/// `[reconnection]` default block. The user uncomments the channels they want
/// and fills in ports.
async fn handle_generate(
  ssh_config: Option<std::path::PathBuf>,
  output: Option<std::path::PathBuf>,
) -> AnyhowResult<()> {
  let ssh_config_path = ssh_config.unwrap_or_else(default_ssh_config_path);

  ui::header("📝", "Generating config.toml scaffold");
  ui::kv_dim("SSH config", ssh_config_path.display());

  let entries = parse_ssh_config(&ssh_config_path).context("Failed to parse SSH config file")?;

  if entries.is_empty() {
    ui::warn(format!(
      "No usable Host blocks found in {}",
      ssh_config_path.display()
    ));
    ui::hint("Add at least one Host with HostName and User, then re-run `generate`.");
  }

  let output_path = output.unwrap_or_else(|| {
    std::env::current_dir()
      .unwrap_or_else(|_| std::path::PathBuf::from("."))
      .join("config.toml")
  });

  let scaffold = AppConfig::generate_scaffold(&entries);

  std::fs::write(&output_path, scaffold).context("Failed to write configuration file")?;

  ui::kv("Output", output_path.display());
  ui::kv("Templates", entries.len());

  println!();
  ui::success("Configuration scaffold written.");

  if !entries.is_empty() {
    println!();
    ui::subheader("  Hosts found in SSH config:");
    for entry in &entries {
      let target = entry.hostname.as_deref().unwrap_or("?");
      let key_info = match &entry.identity_file {
        Some(path) => format!("key: {}", path.display()),
        None => "no IdentityFile (password required)".to_string(),
      };
      ui::host_entry_line(
        &entry.host,
        target,
        &key_info,
        entry.identity_file.is_some(),
      );
    }
  }

  let needs_password: Vec<_> = entries
    .iter()
    .filter(|e| e.identity_file.is_none())
    .collect();
  if !needs_password.is_empty() {
    println!();
    ui::warn(format!(
      "{} host(s) have no IdentityFile — fill in [auth.<alias>].password in {}",
      needs_password.len(),
      output_path.display()
    ));
  }

  println!();
  ui::hint("All [[channels]] entries are commented out. Uncomment the ones you need");
  ui::hint("and replace LOCAL_PORT / REMOTE_PORT with concrete ports (or host:port).");

  Ok(())
}

/// Handle test command - verify channels are working
async fn handle_test(config_path: std::path::PathBuf) -> AnyhowResult<()> {
  ui::header("🧪", "Testing channels");
  ui::kv_dim("Config", config_path.display());

  let config = AppConfig::from_file(&config_path).context("Failed to load configuration")?;

  if config.channels.is_empty() {
    println!();
    ui::warn("No channels configured.");
    ui::hint("Run `ssh-channels-hub generate` to scaffold one.");
    return Ok(());
  }

  let total = config.channels.len();
  ui::kv("Channels", total);
  println!();

  let mut passed = 0usize;
  let mut failed = 0usize;
  let mut skipped = 0usize;

  for conn in &config.channels {
    if conn.direction == config::Direction::RemoteToLocal {
      println!(
        "  ⏭ {} (remote→local) — skipped, test only covers local listeners",
        conn.name
      );
      skipped += 1;
      continue;
    }

    let local_host = conn.local.host.as_str();
    let local_port = conn.local.port;
    let remote_addr = format!("{}:{}", conn.remote.host, conn.remote.port);
    let label = format!(
      "{} ({}:{} → {})",
      conn.name, local_host, local_port, remote_addr
    );

    // First check if port is listening
    match test_port_connection(local_host, local_port).await {
      Ok(false) => {
        ui::fail(format!("{} — port not listening", label));
        failed += 1;
        continue;
      }
      Err(e) => {
        ui::fail(format!("{} — error checking port: {}", label, e));
        failed += 1;
        continue;
      }
      Ok(true) => match test_tunnel_connection(local_host, local_port).await {
        Ok(true) => {
          ui::success(format!("{} — tunnel working", label));
          passed += 1;
        }
        Ok(false) => {
          ui::fail(format!(
            "{} — tunnel dead (SSH connection may be broken)",
            label
          ));
          failed += 1;
        }
        Err(e) => {
          ui::fail(format!("{} — error testing tunnel: {}", label, e));
          failed += 1;
        }
      },
    }
  }

  println!();
  ui::subheader("  Summary");
  ui::kv("Passed", passed);
  ui::kv("Failed", failed);
  ui::kv("Skipped", skipped);

  if failed == 0 {
    println!();
    ui::success("All testable channels are working.");
    Ok(())
  } else {
    println!();
    ui::fail(format!("{} channel(s) failed.", failed));
    ui::subheader("  Troubleshooting:");
    ui::hint(format!(
      "Make sure the service is running: ssh-channels-hub status -c {}",
      config_path.display()
    ));
    ui::hint("Check whether local ports are listening: `lsof -i -P -n | grep LISTEN`.");
    ui::hint("Re-run with `--debug` to see SSH session logs.");
    ui::hint("Verify the remote service is reachable from the SSH server itself.");
    Err(anyhow::anyhow!("Some channels failed the connection test"))
  }
}
