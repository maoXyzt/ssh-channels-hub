use crate::config::{AppConfig, ChannelConfig, ChannelTypeParams, Direction};
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
/// When a finite retry cycle is exhausted, the manager starts another cycle
/// after a jittered exponential delay capped at 60 seconds.
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
  /// A permanent configuration, host-key, authentication, or channel error.
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
  pub hostname: String,
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

fn same_ssh_route(left: &ChannelConfig, right: &ChannelConfig) -> bool {
  left.host == right.host
    && left.port == right.port
    && left.username == right.username
    && left.auth == right.auth
    && left.proxy_jumps == right.proxy_jumps
}

fn remote_bind_port(config: &ChannelConfig) -> Option<u16> {
  match config.params {
    ChannelTypeParams::ForwardedTcpIp {
      remote_bind_port, ..
    } if remote_bind_port != 0 => Some(remote_bind_port),
    _ => None,
  }
}

fn can_share_session(group: &[ChannelConfig], candidate: &ChannelConfig) -> bool {
  same_ssh_route(&group[0], candidate)
    && remote_bind_port(candidate).is_none_or(|port| {
      group
        .iter()
        .all(|existing| remote_bind_port(existing) != Some(port))
    })
}

fn group_channels(channels: Vec<ChannelConfig>) -> Vec<Vec<ChannelConfig>> {
  let mut groups: Vec<Vec<ChannelConfig>> = Vec::new();

  // ponytail: O(n²) startup grouping; use a hash key if channel counts become large.
  for channel in channels {
    match groups
      .iter_mut()
      .find(|group| can_share_session(group, &channel))
    {
      Some(group) => group.push(channel),
      None => groups.push(vec![channel]),
    }
  }

  groups
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
    self.config.checked_web_port()?;

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

    for channel_group in group_channels(channels) {
      let mut manager = SshManager::new(channel_group.clone(), self.config.reconnection.clone());

      match manager.start().await {
        Ok(_) => {
          for channel_config in &channel_group {
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
          }
          managers.push(manager);
        }
        Err(e) => {
          for channel_config in &channel_group {
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
    }

    let mut state = self.state.lock().await;
    let mut managers_guard = self.managers.lock().await;
    *managers_guard = managers;

    let active = managers_guard
      .iter()
      .map(SshManager::channel_count)
      .sum::<usize>();
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
    let channels: Vec<ChannelStatus> = managers.iter().flat_map(SshManager::snapshots).collect();
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::config::AuthConfig;
  use std::path::PathBuf;

  fn channel(name: &str, host: &str, key: &str, local_port: u16) -> ChannelConfig {
    ChannelConfig {
      name: name.into(),
      hostname: "server".into(),
      host: host.into(),
      port: 22,
      username: "alice".into(),
      auth: AuthConfig::Key {
        key_path: PathBuf::from(key),
        passphrase: None,
      },
      params: ChannelTypeParams::DirectTcpIp {
        listen_host: "127.0.0.1".into(),
        local_port,
        dest_host: "db.internal".into(),
        dest_port: 5432,
      },
      proxy_jumps: Vec::new(),
    }
  }

  #[test]
  fn groups_only_channels_with_the_same_ssh_route() {
    let mut duplicate_remote_port = channel("duplicate-remote", "server", "/key-a", 28080);
    duplicate_remote_port.params = ChannelTypeParams::ForwardedTcpIp {
      remote_bind_host: "0.0.0.0".into(),
      remote_bind_port: 8080,
      local_connect_host: "127.0.0.1".into(),
      local_connect_port: 8080,
    };
    let mut first_remote = duplicate_remote_port.clone();
    first_remote.name = "first-remote".into();

    let groups = group_channels(vec![
      channel("db", "server", "/key-a", 15432),
      channel("web", "server", "/key-a", 18080),
      first_remote,
      duplicate_remote_port,
      channel("other-auth", "server", "/key-b", 25432),
      channel("other-host", "backup", "/key-a", 35432),
    ]);

    assert_eq!(groups.len(), 4);
    assert_eq!(groups[0].len(), 3);
    assert_eq!(groups[0][0].name, "db");
    assert_eq!(groups[0][1].name, "web");
    assert_eq!(groups[0][2].name, "first-remote");
    assert_eq!(groups[1][0].name, "duplicate-remote");
  }

  #[tokio::test]
  async fn start_rejects_duplicate_local_listeners_before_building_channels() {
    let config: AppConfig = toml::from_str(
      r#"
[[channels]]
name = "first"
hostname = "missing"
direction = "local->remote"
local = "9090"
remote = "80"

[[channels]]
name = "second"
hostname = "missing"
direction = "local->remote"
local = "9090"
remote = "81"
"#,
    )
    .unwrap();

    let error = ServiceManager::new(config).start().await.unwrap_err();
    assert!(error.to_string().contains("local listener conflict"));
  }
}
