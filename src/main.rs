mod cli;
mod config;
mod error;
mod host_check;
mod port_check;
mod service;
mod ssh;
mod ssh_config;
mod ui;
mod web;

use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
use clap::Parser;
use cli::{Cli, Commands, HostOutputFormat};
use config::AppConfig;
use host_check::{HostSupportReport, HostSupportStatus, analyze_hosts};
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
    Commands::Status { watch, interval } => {
      handle_status(config_path, watch, interval).await?;
    }
    Commands::Validate { config } => {
      let path = config.or(Some(config_path));
      handle_validate(path).await?;
    }
    Commands::Generate { ssh_config, output } => {
      handle_generate(ssh_config, output).await?;
    }
    Commands::Hosts { ssh_config, format } => {
      handle_hosts(ssh_config, format).await?;
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
  let web_enabled = AppConfig::from_file(config_path)
    .map(|config| config.web.enabled)
    .unwrap_or(false);
  if web_enabled {
    let _ = std::fs::remove_file(web_port_file_path(config_path));
  }
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
  if web_enabled {
    match wait_for_web_port(config_path).await {
      Ok(port) => ui::info(format!("Web status: http://127.0.0.1:{}", port)),
      Err(error) => ui::warn(format!("Web status address unavailable: {}", error)),
    }
  }
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

  let web_config = config.web.clone();
  let service_manager = Arc::new(ServiceManager::new(config));

  // Start the service
  service_manager
    .start()
    .await
    .context("Failed to start service")?;

  let cancel = CancellationToken::new();
  let web_port = if web_config.enabled {
    Some(
      web::start(&web_config, Arc::clone(&service_manager), cancel.clone())
        .await
        .context("Failed to start Web status page")?,
    )
  } else {
    None
  };

  // Start IPC listener so "status" command can query this process
  let port = start_ipc_listener(&config_path, Arc::clone(&service_manager), cancel.clone())
    .await
    .context("Failed to start IPC listener for status queries")?;
  write_pid_file(&pid_file_path(&config_path)).context("Write PID file")?;
  debug!(
    "Status query listener on 127.0.0.1:{} (status command will connect here)",
    port
  );

  if let Some(web_port) = web_port {
    write_web_port(&web_port_file_path(&config_path), web_port).context("Write Web port file")?;
    ui::info(format!("Web status: http://127.0.0.1:{}", web_port));
  } else {
    let _ = std::fs::remove_file(web_port_file_path(&config_path));
  }

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

fn web_port_file_path(config_path: &Path) -> PathBuf {
  run_dir(config_path).join("ssh-channels-hub.web.port")
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

fn write_web_port(path: &Path, port: u16) -> AnyhowResult<()> {
  std::fs::write(path, port.to_string()).context("Write Web port file")?;
  Ok(())
}

fn read_web_port(config_path: &Path) -> AnyhowResult<u16> {
  std::fs::read_to_string(web_port_file_path(config_path))
    .context("Read Web port file")?
    .trim()
    .parse()
    .context("Parse Web port file")
}

async fn wait_for_web_port(config_path: &Path) -> AnyhowResult<u16> {
  for _ in 0..20 {
    if let Ok(port) = read_web_port(config_path) {
      return Ok(port);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  read_web_port(config_path)
}

fn remove_run_files(config_path: &Path) -> AnyhowResult<()> {
  for path in [
    pid_file_path(config_path),
    port_file_path(config_path),
    web_port_file_path(config_path),
  ] {
    if path.exists() {
      let _ = std::fs::remove_file(&path);
    }
  }
  Ok(())
}

/// Wire format for the IPC `status` reply. Distinct from `ServiceStatus` so
/// the runtime types can carry richer enums (with payload) while the on-wire
/// schema stays TOML-friendly.
#[derive(serde::Serialize, serde::Deserialize)]
struct ServiceStatusWire {
  state: String,
  #[serde(default)]
  channels: Vec<ChannelStatusWire>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ChannelStatusWire {
  name: String,
  direction: String, // "local->remote" / "remote->local"
  local: String,
  remote: String,
  health: String, // "Stopped" / "Connecting" / "Connected" / "Reconnecting" / "Failed"
  #[serde(default)]
  attempt: u32, // 0 when N/A
  #[serde(default)]
  last_error: String, // "" when N/A
}

fn service_state_label(s: &ServiceState) -> &'static str {
  match s {
    ServiceState::Running => "Running",
    ServiceState::Stopped => "Stopped",
    ServiceState::Starting => "Starting",
    ServiceState::Stopping => "Stopping",
    ServiceState::Error(_) => "Error",
  }
}

fn channel_status_to_wire(c: &service::ChannelStatus) -> ChannelStatusWire {
  let (health, attempt, last_error) = match &c.health {
    service::ChannelHealth::Stopped => ("Stopped", 0, String::new()),
    service::ChannelHealth::Connecting { attempt } => ("Connecting", *attempt, String::new()),
    service::ChannelHealth::Connected => ("Connected", 0, String::new()),
    service::ChannelHealth::Reconnecting {
      attempt,
      last_error,
    } => ("Reconnecting", *attempt, last_error.clone()),
    service::ChannelHealth::Failed { error } => ("Failed", 0, error.clone()),
  };
  ChannelStatusWire {
    name: c.name.clone(),
    direction: c.direction.as_arrow().to_string(),
    local: c.local.clone(),
    remote: c.remote.clone(),
    health: health.to_string(),
    attempt,
    last_error,
  }
}

fn wire_to_channel_status(w: ChannelStatusWire) -> AnyhowResult<service::ChannelStatus> {
  use crate::config::Direction;
  let direction = match w.direction.as_str() {
    "local->remote" => Direction::LocalToRemote,
    "remote->local" => Direction::RemoteToLocal,
    other => return Err(anyhow::anyhow!("Unknown direction in IPC: {}", other)),
  };
  let health = match w.health.as_str() {
    "Stopped" => service::ChannelHealth::Stopped,
    "Connecting" => service::ChannelHealth::Connecting { attempt: w.attempt },
    "Connected" => service::ChannelHealth::Connected,
    "Reconnecting" => service::ChannelHealth::Reconnecting {
      attempt: w.attempt,
      last_error: w.last_error,
    },
    "Failed" => service::ChannelHealth::Failed {
      error: w.last_error,
    },
    other => return Err(anyhow::anyhow!("Unknown health in IPC: {}", other)),
  };
  Ok(service::ChannelStatus {
    name: w.name,
    direction,
    local: w.local,
    remote: w.remote,
    health,
  })
}

/// Serialize ServiceStatus to TOML (one-way protocol: server sends, client reads).
fn status_to_toml(status: &service::ServiceStatus) -> String {
  let wire = ServiceStatusWire {
    state: service_state_label(&status.state).to_string(),
    channels: status.channels.iter().map(channel_status_to_wire).collect(),
  };
  // Hand-pick toml encoding: the wire struct is intentionally flat so this
  // can't fail in practice; treat any error as an empty TOML so the client
  // doesn't choke on garbage.
  toml::to_string(&wire).unwrap_or_default()
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

fn is_daemon_unreachable(error: &anyhow::Error) -> bool {
  error.chain().any(|cause| {
    let msg = cause.to_string();
    msg.contains("Read port file")
      || msg.contains("Parse port file")
      || msg.contains("Connect to service")
  })
}

fn parse_status_toml(s: &str) -> AnyhowResult<service::ServiceStatus> {
  let r: ServiceStatusWire = toml::from_str(s).context("Parse status TOML")?;
  let state = match r.state.as_str() {
    "Running" => ServiceState::Running,
    "Stopped" => ServiceState::Stopped,
    "Starting" => ServiceState::Starting,
    "Stopping" => ServiceState::Stopping,
    "Error" => ServiceState::Error(String::new()),
    other => return Err(anyhow::anyhow!("Unknown state: {}", other)),
  };
  let channels: AnyhowResult<Vec<_>> = r.channels.into_iter().map(wire_to_channel_status).collect();
  Ok(service::ServiceStatus {
    state,
    channels: channels?,
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

/// What `render_status_once` produces — the bits needed to draw one frame.
struct StatusFrame {
  status: service::ServiceStatus,
  pid: Option<String>,
  note: Option<String>,
  config_missing: bool,
  config_error: Option<String>,
}

/// Compose the next status frame. Tries IPC first; falls back to reading
/// config.toml and showing each channel as `Stopped`. Both paths are
/// non-blocking enough for the watch loop.
async fn render_status_once(config_path: &Path) -> StatusFrame {
  // IPC fast path — daemon is alive
  match query_status_via_ipc(config_path).await {
    Ok(status) => {
      let pid = std::fs::read_to_string(pid_file_path(config_path))
        .ok()
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
      return StatusFrame {
        status,
        pid,
        note: None,
        config_missing: false,
        config_error: None,
      };
    }
    Err(e) if !is_daemon_unreachable(&e) => {
      return StatusFrame {
        status: service::ServiceStatus {
          state: ServiceState::Stopped,
          channels: Vec::new(),
        },
        pid: None,
        note: None,
        config_missing: false,
        config_error: Some(format!("Failed to query daemon status: {}", e)),
      };
    }
    Err(_) => {}
  }

  if !config_path.exists() {
    return StatusFrame {
      status: service::ServiceStatus {
        state: ServiceState::Stopped,
        channels: Vec::new(),
      },
      pid: None,
      note: None,
      config_missing: true,
      config_error: None,
    };
  }

  match AppConfig::from_file(config_path) {
    Ok(config) => {
      // Build a Stopped ServiceStatus from the declared channels so the
      // renderer can show the topology the user is going to bring up.
      let channels: Vec<service::ChannelStatus> = config
        .channels
        .iter()
        .map(|c| service::ChannelStatus {
          name: c.name.clone(),
          direction: c.direction,
          local: format!("{}:{}", c.local.host, c.local.port),
          remote: format!("{}:{}", c.remote.host, c.remote.port),
          health: service::ChannelHealth::Stopped,
        })
        .collect();
      StatusFrame {
        status: service::ServiceStatus {
          state: ServiceState::Stopped,
          channels,
        },
        pid: None,
        note: Some("Service is not running. Start with: `ssh-channels-hub start -D`".to_string()),
        config_missing: false,
        config_error: None,
      }
    }
    Err(e) => StatusFrame {
      status: service::ServiceStatus {
        state: ServiceState::Stopped,
        channels: Vec::new(),
      },
      pid: None,
      note: None,
      config_missing: false,
      config_error: Some(e.to_string()),
    },
  }
}

/// Draw one status frame to stdout. Returns `Err` only when the underlying
/// config is unreadable in a way the user should know about; `Ok` covers
/// "everything fine" and "service stopped, no daemon".
fn draw_status_frame(frame: &StatusFrame, config_path: &Path) -> AnyhowResult<()> {
  if frame.config_missing {
    ui::header("📋", "Service Status");
    ui::fail(format!(
      "Configuration file not found: {}",
      config_path.display()
    ));
    ui::hint("Run `ssh-channels-hub generate` to scaffold a config.toml from ~/.ssh/config.");
    return Ok(());
  }
  if let Some(err) = &frame.config_error {
    ui::header("📋", "Service Status");
    ui::fail(format!("Failed to load configuration: {}", err));
    return Err(anyhow::anyhow!("Failed to load config: {}", err));
  }
  ui::print_service_status(
    &frame.status,
    config_path,
    frame.pid.as_deref(),
    frame.note.as_deref(),
  );
  Ok(())
}

/// ANSI clear screen + cursor-home. Used at the start of each watch frame so
/// the new render replaces the previous one in place.
const ANSI_CLEAR_HOME: &str = "\x1b[2J\x1b[H";

/// Handle status command: one-shot render by default; with `--watch`, re-render
/// every `interval` seconds until Ctrl+C.
async fn handle_status(config_path: PathBuf, watch: bool, interval: u64) -> AnyhowResult<()> {
  if !watch {
    let frame = render_status_once(&config_path).await;
    return draw_status_frame(&frame, &config_path);
  }

  let interval = interval.max(1);
  let mut ticker = tokio::time::interval(Duration::from_secs(interval));
  // If a render takes longer than `interval`, drop the missed ticks rather
  // than burst-firing — we always want one render per real-time interval.
  ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

  use std::io::Write;
  loop {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            // Leave the user's terminal in a clean state on exit.
            println!();
            ui::info("Exited watch mode.");
            return Ok(());
        }
        _ = ticker.tick() => {}
    }

    let frame = render_status_once(&config_path).await;
    // Best-effort clear; if the terminal doesn't support ANSI (e.g. legacy
    // cmd.exe), output appends and the user still sees the latest frame.
    print!("{}", ANSI_CLEAR_HOME);
    // In watch mode we swallow config-load errors — the frame's already been
    // rendered (the error path inside draw_status_frame prints before
    // returning) and we want to keep polling so the user sees recovery the
    // moment they fix the file.
    let _ = draw_status_frame(&frame, &config_path);
    println!();
    ui::hint(format!(
      "↻ Refreshing every {}s. Press Ctrl+C to exit.",
      interval
    ));
    let _ = std::io::stdout().flush();
  }
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

/// Handle hosts command: report whether SSH config aliases are usable here.
async fn handle_hosts(
  ssh_config: Option<std::path::PathBuf>,
  format: HostOutputFormat,
) -> AnyhowResult<()> {
  let ssh_config_path = ssh_config.unwrap_or_else(default_ssh_config_path);
  let entries = parse_ssh_config(&ssh_config_path).context("Failed to parse SSH config file")?;
  let reports = analyze_hosts(&entries);

  match format {
    HostOutputFormat::Json => {
      println!(
        "{}",
        serde_json::to_string_pretty(&reports).context("Serialize host report as JSON")?
      );
    }
    HostOutputFormat::Table => {
      render_hosts_table(&ssh_config_path, &reports);
    }
  }

  Ok(())
}

fn render_hosts_table(ssh_config_path: &Path, reports: &[HostSupportReport]) {
  ui::header("🖥️", "SSH host support");
  ui::kv_dim("SSH config", ssh_config_path.display());
  ui::kv("Hosts", reports.len());

  if reports.is_empty() {
    println!();
    ui::warn("No Host blocks found.");
    ui::hint("Add Host aliases to SSH config, then re-run `ssh-channels-hub hosts`.");
    return;
  }

  println!();
  for report in reports {
    let target = report.hostname.as_deref().unwrap_or("?");
    let status = match report.status {
      HostSupportStatus::Supported => "supported",
      HostSupportStatus::Unsupported => "unsupported",
    };

    println!("  [{}] -> {}  {}", report.alias, target, status);

    match report.status {
      HostSupportStatus::Supported => {
        for warning in &report.warnings {
          println!("      warning: {}", warning);
        }
      }
      HostSupportStatus::Unsupported => {
        for reason in &report.reasons {
          println!("      reason: {}", reason);
        }
      }
    }
  }
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::config::Direction;
  use crate::service::{ChannelHealth, ChannelStatus, ServiceState, ServiceStatus};

  fn ch(name: &str, health: ChannelHealth) -> ChannelStatus {
    ChannelStatus {
      name: name.to_string(),
      direction: Direction::LocalToRemote,
      local: "127.0.0.1:3306".to_string(),
      remote: "db.internal:3306".to_string(),
      health,
    }
  }

  fn round_trip(status: ServiceStatus) -> ServiceStatus {
    let toml = status_to_toml(&status);
    parse_status_toml(&toml).expect("parse round-trip")
  }

  #[test]
  fn status_roundtrips_connected_channel() {
    let s = ServiceStatus {
      state: ServiceState::Running,
      channels: vec![ch("db", ChannelHealth::Connected)],
    };
    let r = round_trip(s);
    assert_eq!(r.state, ServiceState::Running);
    assert_eq!(r.channels.len(), 1);
    assert!(matches!(r.channels[0].health, ChannelHealth::Connected));
    assert_eq!(r.channels[0].name, "db");
    assert_eq!(r.channels[0].direction, Direction::LocalToRemote);
  }

  #[test]
  fn status_roundtrips_reconnecting_with_attempt_and_error() {
    let s = ServiceStatus {
      state: ServiceState::Running,
      channels: vec![ch(
        "web",
        ChannelHealth::Reconnecting {
          attempt: 7,
          last_error: "connection refused".to_string(),
        },
      )],
    };
    let r = round_trip(s);
    match &r.channels[0].health {
      ChannelHealth::Reconnecting {
        attempt,
        last_error,
      } => {
        assert_eq!(*attempt, 7);
        assert_eq!(last_error, "connection refused");
      }
      other => panic!("expected Reconnecting, got {:?}", other),
    }
  }

  #[test]
  fn status_roundtrips_failed_with_error() {
    let s = ServiceStatus {
      state: ServiceState::Error("boom".to_string()),
      channels: vec![ch(
        "db",
        ChannelHealth::Failed {
          error: "auth failed".to_string(),
        },
      )],
    };
    let r = round_trip(s);
    // ServiceState::Error message isn't preserved on the wire — only the kind
    // is. That's intentional (the wire stays flat), document via assert.
    assert_eq!(r.state, ServiceState::Error(String::new()));
    match &r.channels[0].health {
      ChannelHealth::Failed { error } => assert_eq!(error, "auth failed"),
      other => panic!("expected Failed, got {:?}", other),
    }
  }

  #[test]
  fn status_roundtrips_connecting_with_attempt() {
    let s = ServiceStatus {
      state: ServiceState::Starting,
      channels: vec![ch("db", ChannelHealth::Connecting { attempt: 3 })],
    };
    let r = round_trip(s);
    match &r.channels[0].health {
      ChannelHealth::Connecting { attempt } => assert_eq!(*attempt, 3),
      other => panic!("expected Connecting, got {:?}", other),
    }
  }

  #[test]
  fn status_roundtrips_stopped_with_empty_channels() {
    let s = ServiceStatus {
      state: ServiceState::Stopped,
      channels: vec![],
    };
    let r = round_trip(s);
    assert_eq!(r.state, ServiceState::Stopped);
    assert!(r.channels.is_empty());
  }

  #[test]
  fn parse_rejects_unknown_state() {
    let bogus = r#"state = "Levitating"
"#;
    assert!(parse_status_toml(bogus).is_err());
  }

  #[test]
  fn parse_rejects_unknown_health() {
    let bogus = r#"state = "Running"
[[channels]]
name = "x"
direction = "local->remote"
local = "127.0.0.1:1"
remote = "127.0.0.1:2"
health = "Floating"
"#;
    assert!(parse_status_toml(bogus).is_err());
  }

  #[test]
  fn parse_rejects_unknown_direction() {
    let bogus = r#"state = "Running"
[[channels]]
name = "x"
direction = "sideways"
local = "127.0.0.1:1"
remote = "127.0.0.1:2"
health = "Connected"
"#;
    assert!(parse_status_toml(bogus).is_err());
  }

  #[tokio::test]
  async fn render_status_reports_daemon_protocol_errors() {
    let unique = format!(
      "ssh-channels-hub-status-test-{}-{}",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("config.toml");
    std::fs::write(&config_path, "[reconnection]\n").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    write_port_file(&port_file_path(&config_path), port).unwrap();
    let server = tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let _ = read_line_async(&mut stream).await.unwrap();
      stream.write_all(b"not valid toml").await.unwrap();
      stream.shutdown().await.unwrap();
    });

    let frame = render_status_once(&config_path).await;
    server.await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
      frame
        .config_error
        .as_deref()
        .is_some_and(|e| e.contains("Failed to query daemon status")),
      "expected daemon query error, got {:?}",
      frame.config_error
    );
  }

  #[test]
  fn parse_back_compat_defaults_attempt_and_error_to_zero_and_empty() {
    // Minimal channel TOML — attempt/last_error omitted — should still parse
    // (serde defaults). Ensures the wire stays forgiving across versions.
    let minimal = r#"state = "Running"
[[channels]]
name = "x"
direction = "local->remote"
local = "127.0.0.1:1"
remote = "127.0.0.1:2"
health = "Connected"
"#;
    let r = parse_status_toml(minimal).expect("minimal channel parses");
    assert_eq!(r.channels.len(), 1);
    assert!(matches!(r.channels[0].health, ChannelHealth::Connected));
  }
}
