use crate::config::{AppConfig, ChannelTypeParams, Direction};
use crate::error::{AppError, Result};
use crate::port_check::check_ports;
use crate::ssh::SshManager;
use crate::ui;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// Service state
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceState {
  Stopped,
  Starting,
  Running,
  Stopping,
  Error(String),
}

/// Per-channel runtime health, written by the SSH manager task and read by
/// `status` / `--watch`. Distinct from `ServiceState`, which is service-wide.
///
/// Transition map (rough):
///   Stopped → Connecting{1} → Connected → (session drop)
///           → Reconnecting{n} → Connecting{n+1} → Connected …
/// When `max_retries > 0` is exhausted the state lands in `Failed`; otherwise
/// the outer `SshManager::start` loop kicks off a fresh `Connecting{1}` cycle
/// after a 1 s pause.
#[derive(Debug, Clone, Default)]
pub enum ChannelHealth {
  /// Manager hasn't started its connect loop yet, or has stopped after cancel.
  #[default]
  Stopped,
  /// First connect attempt in this cycle, or after a successful retry transition.
  /// `attempt` starts at 1 within a cycle.
  Connecting { attempt: u32 },
  /// SSH session is up AND the channel-side setup (listener bind / tcpip-forward)
  /// completed. The reconnect loop is blocked waiting for the session to end.
  Connected,
  /// A previous attempt failed; we're inside backon's backoff before retrying.
  Reconnecting { attempt: u32, last_error: String },
  /// `max_retries` was non-zero and is now exhausted. The outer loop may still
  /// restart the cycle (a project quirk — see `SshManager::start`).
  Failed { error: String },
}

impl ChannelHealth {
  /// True when the SSH session is up and the channel is actively serving.
  /// `status` uses this for the aggregate `connected / total` ratio.
  pub fn is_connected(&self) -> bool {
    matches!(self, ChannelHealth::Connected)
  }
}

/// One row in `ServiceStatus.channels` — captures everything `status` and
/// `--watch` need to render a channel without re-reading config.toml.
#[derive(Debug, Clone)]
pub struct ChannelStatus {
  pub name: String,
  pub direction: Direction,
  pub local: String,
  pub remote: String,
  pub health: ChannelHealth,
}

/// Service manager that manages all SSH channels
pub struct ServiceManager {
  config: AppConfig,
  state: Arc<Mutex<ServiceState>>,
  managers: Arc<Mutex<Vec<SshManager>>>,
}

impl ServiceManager {
  /// Create a new service manager
  pub fn new(config: AppConfig) -> Self {
    Self {
      config,
      state: Arc::new(Mutex::new(ServiceState::Stopped)),
      managers: Arc::new(Mutex::new(Vec::new())),
    }
  }

  /// Start the service
  pub async fn start(&self) -> Result<()> {
    let mut state = self.state.lock().await;

    if *state != ServiceState::Stopped {
      return Err(AppError::Service(format!(
        "Service is not stopped (current state: {:?})",
        *state
      )));
    }

    *state = ServiceState::Starting;
    drop(state);

    info!("Starting SSH Channels Hub service");

    // Check port availability before starting channels
    let binds_to_check: Vec<(String, u16)> = self
      .config
      .channels
      .iter()
      .filter_map(|conn| conn.local_listen_bind())
      .collect();

    if !binds_to_check.is_empty() {
      info!(
        "Checking port availability for {} bind(s)",
        binds_to_check.len()
      );
      match check_ports(&binds_to_check).await {
        Ok(occupied) => {
          if !occupied.is_empty() {
            let listing = occupied
              .iter()
              .map(|(h, p)| format!("{}:{}", h, p))
              .collect::<Vec<_>>()
              .join(", ");
            let error_msg = format!(
              "Address(es) already in use: {}. Please stop the application using these ports or change the configuration.",
              listing
            );
            error!(occupied = %listing, "Port check failed");
            let mut state = self.state.lock().await;
            *state = ServiceState::Error(error_msg.clone());
            return Err(AppError::Service(error_msg));
          }
          info!("All ports are available");
        }
        Err(e) => {
          warn!(error = ?e, "Failed to check port availability, continuing anyway");
          // Continue even if port check fails (might be a permission issue)
        }
      }
    }

    let mut managers = Vec::new();
    let mut errors = Vec::new();

    let channels = self
      .config
      .build_channels()
      .map_err(|e| AppError::Service(format!("Failed to build channels: {}", e)))?;

    info!("Found {} channel(s) to start", channels.len());

    for channel_config in channels {
      let mut manager = SshManager::new(channel_config.clone(), self.config.reconnection.clone());

      match manager.start().await {
        Ok(_) => {
          match &channel_config.params {
            ChannelTypeParams::ForwardedTcpIp {
              remote_bind_host,
              remote_bind_port,
              local_connect_host,
              local_connect_port,
            } => {
              let remote = format!("{}:{}", remote_bind_host, remote_bind_port);
              let local_dest = format!("{}:{}", local_connect_host, local_connect_port);
              ui::success(format!(
                "{}  remote {} ← local {}  via {}@{}",
                channel_config.name,
                remote,
                local_dest,
                channel_config.username,
                channel_config.host
              ));
            }
            ChannelTypeParams::DirectTcpIp {
              listen_host,
              local_port,
              dest_host,
              dest_port,
            } => {
              ui::success(format!(
                "{}  local {}:{} → remote {}:{}  via {}@{}",
                channel_config.name,
                listen_host,
                local_port,
                dest_host,
                dest_port,
                channel_config.username,
                channel_config.host
              ));
            }
          }

          info!(channel = %channel_config.name, "Started SSH manager");
          managers.push(manager);
        }
        Err(e) => {
          ui::fail(format!("{} — {}", channel_config.name, e));
          error!(
              channel = %channel_config.name,
              error = ?e,
              "Failed to start SSH manager"
          );
          errors.push(format!("{}: {}", channel_config.name, e));
        }
      }
    }

    let mut state = self.state.lock().await;
    let mut managers_guard = self.managers.lock().await;
    *managers_guard = managers;

    let active = managers_guard.len();
    let total = active + errors.len();

    if errors.is_empty() {
      *state = ServiceState::Running;
      println!();
      ui::success(format!(
        "Service started — {}/{} channel(s) active.",
        active, total
      ));
      info!("Service started successfully");
      Ok(())
    } else if managers_guard.is_empty() {
      *state = ServiceState::Error(format!("All channels failed: {}", errors.join(", ")));
      println!();
      ui::fail(format!(
        "Service failed to start — all {} channel(s) errored.",
        errors.len()
      ));
      Err(AppError::Service(format!(
        "Failed to start any channels: {}",
        errors.join(", ")
      )))
    } else {
      *state = ServiceState::Running;
      println!();
      ui::warn(format!(
        "Service started with errors — {} active, {} failed.",
        active,
        errors.len()
      ));
      warn!(
          errors = %errors.join(", "),
          "Service started with some channel failures"
      );
      Ok(())
    }
  }

  /// Stop the service
  pub async fn stop(&self) -> Result<()> {
    let mut state = self.state.lock().await;

    if *state != ServiceState::Running {
      return Err(AppError::Service(format!(
        "Service is not running (current state: {:?})",
        *state
      )));
    }

    *state = ServiceState::Stopping;
    drop(state);

    info!("Stopping SSH Channels Hub service");

    let mut managers = self.managers.lock().await;
    let mut errors = Vec::new();

    for manager in managers.iter_mut() {
      if let Err(e) = manager.stop().await {
        error!(error = ?e, "Failed to stop SSH manager");
        errors.push(e.to_string());
      }
    }

    managers.clear();

    let mut state = self.state.lock().await;
    *state = ServiceState::Stopped;

    if errors.is_empty() {
      info!("Service stopped successfully");
      Ok(())
    } else {
      warn!(errors = %errors.join(", "), "Service stopped with some errors");
      Ok(())
    }
  }

  // /// Restart the service
  // pub async fn restart(&self) -> Result<()> {
  //     info!("Restarting SSH Channels Hub service");
  //     self.stop().await?;
  //     tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
  //     self.start().await
  // }

  /// Get service status — walks every running `SshManager` and snapshots its
  /// live health, so `status` reflects what's actually connected (not just
  /// what's been spawned).
  pub async fn status(&self) -> ServiceStatus {
    let state = self.state.lock().await.clone();
    let managers = self.managers.lock().await;
    let channels: Vec<ChannelStatus> = managers.iter().map(|m| m.snapshot()).collect();
    ServiceStatus { state, channels }
  }
}

/// Service status information
#[derive(Debug, Clone)]
pub struct ServiceStatus {
  pub state: ServiceState,
  pub channels: Vec<ChannelStatus>,
}

impl ServiceStatus {
  /// Count of channels currently in `Connected` health.
  pub fn connected_count(&self) -> usize {
    self
      .channels
      .iter()
      .filter(|c| c.health.is_connected())
      .count()
  }

  /// Total channels known to the service.
  pub fn total_count(&self) -> usize {
    self.channels.len()
  }
}

impl std::fmt::Display for ServiceStatus {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "State: {:?}, Channels: {}/{}",
      self.state,
      self.connected_count(),
      self.total_count()
    )
  }
}
