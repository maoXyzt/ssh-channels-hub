use crate::config::{AuthConfig, ChannelConfig, ChannelTypeParams, Direction, ReconnectionConfig};
use crate::error::{AppError, Result};
use crate::service::{ChannelHealth, ChannelStatus};
use backon::{BackoffBuilder, ExponentialBackoff, ExponentialBuilder, Retryable};
use russh::*;
use russh_keys::key::KeyPair;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

const EXHAUSTED_RETRY_MAX_DELAY: Duration = Duration::from_secs(60);

// ponytail: one global permit favors server safety; use per-route semaphores if
// unrelated hosts need parallel handshakes.
static SSH_HANDSHAKE_LIMIT: Semaphore = Semaphore::const_new(1);

/// SSH client handler for ProxyJump hops.
///
/// Uses the jump hop's resolved host and port for strict `known_hosts`
/// verification. There is no trust-on-first-use because this is a
/// non-interactive daemon.
#[derive(Clone)]
struct JumpClientHandler {
  alias: String,
  host: String,
  port: u16,
}

#[async_trait::async_trait]
impl client::Handler for JumpClientHandler {
  type Error = russh::Error;

  async fn check_server_key(
    &mut self,
    server_public_key: &russh_keys::key::PublicKey,
  ) -> std::result::Result<bool, Self::Error> {
    let home = dirs::home_dir();
    let known_hosts_path = home.as_ref().map(|h| h.join(".ssh").join("known_hosts"));
    let known_hosts_path_display = known_hosts_path
      .as_ref()
      .map(|p| p.display().to_string())
      .unwrap_or_else(|| "<home-not-found>".to_string());
    let env_home = std::env::var("HOME").unwrap_or_else(|_| "<unset>".to_string());
    let server_key_algorithm = server_public_key.name();
    let server_key_fingerprint = server_public_key.fingerprint();

    match &known_hosts_path {
      Some(path) => match russh_keys::known_host_keys_path(&self.host, self.port, path) {
        Ok(keys) => {
          let recorded_keys: Vec<String> = keys
            .iter()
            .map(|(line, key)| format!("line {}: {} {}", line, key.name(), key.fingerprint()))
            .collect();
          debug!(
            alias = %self.alias,
            host = %self.host,
            port = self.port,
            env_home = %env_home,
            home = ?home,
            known_hosts = %known_hosts_path_display,
            server_key_algorithm = %server_key_algorithm,
            server_key_fingerprint = %server_key_fingerprint,
            known_hosts_matches = recorded_keys.len(),
            recorded_keys = ?recorded_keys,
            "Checking ProxyJump server key against known_hosts"
          );
        }
        Err(e) => {
          debug!(
            alias = %self.alias,
            host = %self.host,
            port = self.port,
            env_home = %env_home,
            home = ?home,
            known_hosts = %known_hosts_path_display,
            server_key_algorithm = %server_key_algorithm,
            server_key_fingerprint = %server_key_fingerprint,
            error = ?e,
            "Failed to inspect known_hosts before ProxyJump server key check"
          );
        }
      },
      None => {
        debug!(
          alias = %self.alias,
          host = %self.host,
          port = self.port,
          env_home = %env_home,
          server_key_algorithm = %server_key_algorithm,
          server_key_fingerprint = %server_key_fingerprint,
          "Cannot inspect known_hosts before ProxyJump server key check because home directory was not found"
        );
      }
    }

    match russh_keys::check_known_hosts(&self.host, self.port, server_public_key) {
      Ok(true) => Ok(true),
      Ok(false) => {
        error!(
          alias = %self.alias,
          host = %self.host,
          port = self.port,
          env_home = %env_home,
          home = ?home,
          known_hosts = %known_hosts_path_display,
          server_key_algorithm = %server_key_algorithm,
          server_key_fingerprint = %server_key_fingerprint,
          "ProxyJump host not in known_hosts; refusing. Run \
           `ssh-keyscan -p {} {} >> ~/.ssh/known_hosts` or `ssh {}` once \
           to trust it.",
          self.port, self.host, self.alias
        );
        Ok(false)
      }
      Err(russh_keys::Error::KeyChanged { line }) => {
        error!(
          alias = %self.alias,
          host = %self.host,
          port = self.port,
          known_hosts_line = line,
          env_home = %env_home,
          home = ?home,
          known_hosts = %known_hosts_path_display,
          server_key_algorithm = %server_key_algorithm,
          server_key_fingerprint = %server_key_fingerprint,
          "ProxyJump host key changed since last contact (possible MITM). \
           Refusing. Verify out-of-band, then remove the stale line from \
           ~/.ssh/known_hosts."
        );
        Ok(false)
      }
      Err(e) => {
        error!(
          alias = %self.alias,
          host = %self.host,
          port = self.port,
          env_home = %env_home,
          home = ?home,
          known_hosts = %known_hosts_path_display,
          server_key_algorithm = %server_key_algorithm,
          server_key_fingerprint = %server_key_fingerprint,
          error = ?e,
          "known_hosts check failed for ProxyJump"
        );
        Ok(false)
      }
    }
  }
}

/// SSH client handler for direct-tcpip (local forwarding)
#[derive(Clone)]
struct ClientHandler {
  channels: String,
  host: String,
  port: u16,
  via: String,
  reverse_routes: ReverseRoutes,
}

#[derive(Clone)]
struct ReverseDestination {
  channel_name: String,
  local_addr: String,
}

type ReverseRoutes = Arc<StdRwLock<HashMap<u32, ReverseDestination>>>;

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
  type Error = russh::Error;

  async fn check_server_key(
    &mut self,
    server_public_key: &russh_keys::key::PublicKey,
  ) -> std::result::Result<bool, Self::Error> {
    let known_hosts_path = dirs::home_dir()
      .map(|home| home.join(".ssh").join("known_hosts"))
      .map(|path| path.display().to_string())
      .unwrap_or_else(|| "<home-not-found>".to_string());
    let server_key_algorithm = server_public_key.name();
    let server_key_fingerprint = server_public_key.fingerprint();

    match russh_keys::check_known_hosts(&self.host, self.port, server_public_key) {
      Ok(true) => Ok(true),
      Ok(false) => {
        error!(
          channels = %self.channels,
          host = %self.host,
          port = self.port,
          via = %self.via,
          known_hosts = %known_hosts_path,
          server_key_algorithm = %server_key_algorithm,
          server_key_fingerprint = %server_key_fingerprint,
          "SSH target key is not trusted; refusing. Verify the fingerprint, then run \
           `ssh-keyscan -p {} {} >> ~/.ssh/known_hosts`.",
          self.port, self.host
        );
        Ok(false)
      }
      Err(russh_keys::Error::KeyChanged { line }) => {
        error!(
          channels = %self.channels,
          host = %self.host,
          port = self.port,
          via = %self.via,
          known_hosts = %known_hosts_path,
          known_hosts_line = line,
          server_key_algorithm = %server_key_algorithm,
          server_key_fingerprint = %server_key_fingerprint,
          "SSH target host key changed; refusing. Verify the fingerprint out-of-band, \
           then remove the stale known_hosts line."
        );
        Ok(false)
      }
      Err(error) => {
        error!(
          channels = %self.channels,
          host = %self.host,
          port = self.port,
          via = %self.via,
          known_hosts = %known_hosts_path,
          server_key_algorithm = %server_key_algorithm,
          server_key_fingerprint = %server_key_fingerprint,
          error = ?error,
          "SSH target known_hosts check failed"
        );
        Ok(false)
      }
    }
  }

  async fn server_channel_open_forwarded_tcpip(
    &mut self,
    channel: russh::Channel<russh::client::Msg>,
    connected_address: &str,
    connected_port: u32,
    _originator_address: &str,
    _originator_port: u32,
    _session: &mut russh::client::Session,
  ) -> std::result::Result<(), Self::Error> {
    let destination = match self.reverse_routes.read() {
      Ok(routes) => routes.get(&connected_port).cloned(),
      Err(poisoned) => poisoned.into_inner().get(&connected_port).cloned(),
    };
    let Some(destination) = destination else {
      error!(
        connected_address,
        connected_port, "No local destination registered for forwarded-tcpip channel"
      );
      return Ok(());
    };

    match TcpStream::connect(&destination.local_addr).await {
      Ok(mut stream) => {
        let mut channel_stream = channel.into_stream();
        let channel_name = destination.channel_name;
        tokio::spawn(async move {
          if let Err(e) = tokio::io::copy_bidirectional(&mut stream, &mut channel_stream).await {
            debug!(channel = %channel_name, error = ?e, "Forwarded-tcpip relay ended");
          }
        });
      }
      Err(e) => {
        error!(
            channel = %destination.channel_name,
            local = %destination.local_addr,
            error = ?e,
            "Failed to connect to local address for forwarded-tcpip"
        );
      }
    }
    Ok(())
  }
}

/// SSH connection manager
pub struct SshManager {
  configs: Vec<ChannelConfig>,
  reconnection_config: ReconnectionConfig,
  shutdown_tx: Option<mpsc::Sender<()>>,
  cancellation_token: Option<CancellationToken>,
  /// Live channel health, shared with the spawned connection-group loop. Uses
  /// `std::sync::Mutex` because we only hold the lock for state writes
  /// (never across `.await`), and `backon::Retry::notify` takes a sync
  /// closure that needs to do the same.
  health: Vec<Arc<StdMutex<ChannelHealth>>>,
}

impl SshManager {
  /// Create a manager for channels sharing one SSH route and session.
  pub fn new(configs: Vec<ChannelConfig>, reconnection_config: ReconnectionConfig) -> Self {
    let health = configs
      .iter()
      .map(|_| Arc::new(StdMutex::new(ChannelHealth::Stopped)))
      .collect();
    Self {
      configs,
      reconnection_config,
      shutdown_tx: None,
      cancellation_token: None,
      health,
    }
  }

  pub fn channel_count(&self) -> usize {
    self.configs.len()
  }

  /// Snapshots for the `status` command: topology and live health per channel.
  pub fn snapshots(&self) -> impl Iterator<Item = ChannelStatus> + '_ {
    self
      .configs
      .iter()
      .zip(&self.health)
      .map(|(config, health)| channel_status(config, health))
  }

  /// Start managing the shared SSH connection and its channels.
  pub async fn start(&mut self) -> Result<()> {
    let route = self
      .configs
      .first()
      .ok_or_else(|| AppError::Config("SSH manager requires at least one channel".into()))?;
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let cancel = CancellationToken::new();
    self.cancellation_token = Some(cancel.clone());
    self.shutdown_tx = Some(shutdown_tx);

    let configs = self.configs.clone();
    let reconnection_config = self.reconnection_config.clone();
    let health = self.health.clone();
    let route_name = format!("{}@{}:{}", route.username, route.host, route.port);
    let channel_names = self
      .configs
      .iter()
      .map(|config| config.name.as_str())
      .collect::<Vec<_>>()
      .join(", ");

    set_all_health(&health, ChannelHealth::Connecting { attempt: 1 });

    tokio::spawn(async move {
      let initial_delay = Duration::from_secs(reconnection_config.initial_delay_secs)
        .clamp(Duration::from_secs(1), EXHAUSTED_RETRY_MAX_DELAY);
      let mut exhausted_backoff = exhausted_retry_backoff(initial_delay);
      let mut mark_stopped = false;

      loop {
        let result = tokio::select! {
            _ = shutdown_rx.recv() => {
                info!(route = %route_name, "Shutting down SSH manager");
                mark_stopped = true;
                break;
            }
            _ = cancel.cancelled() => {
                mark_stopped = true;
                break;
            }
            result = Self::connect_and_manage_channels(
              &configs,
              &reconnection_config,
              cancel.clone(),
              health.clone()
            ) => result,
        };

        let error = match result {
          Ok(()) if cancel.is_cancelled() => {
            mark_stopped = true;
            break;
          }
          Ok(()) => {
            exhausted_backoff = exhausted_retry_backoff(initial_delay);
            let retry_delay = jittered_retry_delay(initial_delay);
            warn!(
              route = %route_name,
              backoff_ms = retry_delay.as_millis() as u64,
              "Established SSH session ended; starting a fresh retry cycle"
            );
            set_all_health(
              &health,
              ChannelHealth::Reconnecting {
                attempt: 1,
                last_error: "SSH session ended".into(),
              },
            );
            tokio::select! {
              _ = shutdown_rx.recv() => {
                mark_stopped = true;
                break;
              }
              _ = cancel.cancelled() => {
                mark_stopped = true;
                break;
              }
              _ = tokio::time::sleep(retry_delay) => {}
            }
            set_all_health(&health, ChannelHealth::Connecting { attempt: 1 });
            continue;
          }
          Err(error) if !error.is_retryable() => {
            error!(
              channels = %channel_names,
              route = %route_name,
              error = ?error,
              "Permanent SSH error; automatic retries stopped"
            );
            let error_message = error.to_string();
            for channel_health in &health {
              set_failed_unless_already(channel_health, &error_message);
            }
            break;
          }
          Err(error) => error,
        };

        let retry_delay = exhausted_backoff
          .next()
          .expect("exhausted-cycle backoff is configured for unlimited retries");
        warn!(
          route = %route_name,
          backoff_ms = retry_delay.as_millis() as u64,
          error = %error,
          "Retry cycle exhausted; waiting before starting a new cycle"
        );
        set_all_health(
          &health,
          ChannelHealth::Reconnecting {
            attempt: reconnection_config.max_retries.saturating_add(1),
            last_error: error.to_string(),
          },
        );

        tokio::select! {
          _ = shutdown_rx.recv() => {
            mark_stopped = true;
            break;
          }
          _ = cancel.cancelled() => {
            mark_stopped = true;
            break;
          }
          _ = tokio::time::sleep(retry_delay) => {}
        }
        set_all_health(&health, ChannelHealth::Connecting { attempt: 1 });
      }

      if mark_stopped {
        set_all_health(&health, ChannelHealth::Stopped);
      }
    });

    Ok(())
  }

  /// Stop the SSH manager
  pub async fn stop(&mut self) -> Result<()> {
    if let Some(tx) = self.shutdown_tx.take() {
      let _ = tx.send(()).await;
    }
    if let Some(token) = self.cancellation_token.take() {
      token.cancel();
    }
    set_all_health(&self.health, ChannelHealth::Stopped);
    Ok(())
  }

  /// Connect and manage one SSH session shared by compatible channels.
  async fn connect_and_manage_channels(
    configs: &[ChannelConfig],
    reconnection_config: &ReconnectionConfig,
    cancel: CancellationToken,
    health: Vec<Arc<StdMutex<ChannelHealth>>>,
  ) -> Result<()> {
    let route = configs
      .first()
      .expect("SshManager::start rejects empty channel groups");
    let initial_delay =
      Duration::from_secs(reconnection_config.initial_delay_secs).max(Duration::from_secs(1));
    let max_delay = if reconnection_config.use_exponential_backoff {
      Duration::from_secs(reconnection_config.max_delay_secs).max(initial_delay)
    } else {
      initial_delay
    };
    let max_times = if reconnection_config.max_retries == 0 {
      usize::MAX
    } else {
      reconnection_config.max_retries as usize
    };
    let builder = ExponentialBuilder::default()
      .with_min_delay(initial_delay)
      .with_max_delay(max_delay)
      .with_max_times(max_times)
      .with_jitter();

    let attempt_counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let health_for_attempt = health.clone();
    let health_for_notify = health.clone();
    let attempt_for_notify = attempt_counter.clone();

    (|| {
      let n = attempt_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
      set_all_health(
        &health_for_attempt,
        ChannelHealth::Connecting { attempt: n },
      );
      let health = health.clone();
      let cancel = cancel.clone();
      async move { Self::establish_connection(configs, cancel, health).await }
    })
    .retry(&builder)
    .when(AppError::is_retryable)
    .notify(move |err, dur| {
      let attempt = attempt_for_notify.load(std::sync::atomic::Ordering::Relaxed);
      warn!(
          host = %route.host,
          port = route.port,
          attempt,
          backoff_ms = dur.as_millis() as u64,
          error = %err,
          "Connect attempt failed, will retry"
      );
      set_all_health(
        &health_for_notify,
        ChannelHealth::Reconnecting {
          attempt,
          last_error: err.to_string(),
        },
      );
    })
    .await
  }

  /// Establish one SSH connection and serve every configured channel on it.
  async fn establish_connection(
    configs: &[ChannelConfig],
    cancel: CancellationToken,
    health: Vec<Arc<StdMutex<ChannelHealth>>>,
  ) -> Result<()> {
    let route = configs
      .first()
      .expect("SshManager::start rejects empty channel groups");
    if route.proxy_jumps.is_empty() {
      info!(
          host = %route.host,
          port = route.port,
          channels = configs.len(),
          "Establishing shared SSH connection"
      );
    } else {
      let chain: Vec<&str> = route.proxy_jumps.iter().map(|h| h.alias.as_str()).collect();
      info!(
          host = %route.host,
          port = route.port,
          channels = configs.len(),
          via = %chain.join(" -> "),
          "Establishing shared SSH connection through ProxyJump chain"
      );
    }

    let reverse_routes = Arc::new(StdRwLock::new(HashMap::new()));
    let handler = ClientHandler {
      channels: configs
        .iter()
        .map(|config| config.name.as_str())
        .collect::<Vec<_>>()
        .join(", "),
      host: route.host.clone(),
      port: route.port,
      via: route
        .proxy_jumps
        .last()
        .map(|hop| hop.alias.clone())
        .unwrap_or_else(|| "direct".to_string()),
      reverse_routes: reverse_routes.clone(),
    };
    let handshake_permit = SSH_HANDSHAKE_LIMIT
      .acquire()
      .await
      .map_err(|_| AppError::SshConnection("SSH handshake limiter closed".into()))?;
    // `jumps` stays alive for the terminal connection lifetime.
    let (jumps, mut session) = connect_via_chain(route, handler).await?;
    drop(handshake_permit);

    let mut active_reverse_forwards = 0usize;
    for (config, channel_health) in configs.iter().zip(&health) {
      if matches!(&config.params, ChannelTypeParams::ForwardedTcpIp { .. }) {
        match register_forwarded_tcpip(
          &mut session,
          config,
          reverse_routes.clone(),
          channel_health.clone(),
        )
        .await
        {
          Ok(()) => {
            active_reverse_forwards += 1;
          }
          Err(error) if !error.is_retryable() => {
            error!(channel = %config.name, error = ?error, "Remote forward disabled");
            set_health(
              channel_health,
              ChannelHealth::Failed {
                error: error.to_string(),
              },
            );
          }
          Err(error) => return Err(error),
        }
      }
    }

    let session = Arc::new(session);
    let mut channel_tasks = JoinSet::new();
    for (index, (config, channel_health)) in configs.iter().zip(&health).enumerate() {
      if matches!(&config.params, ChannelTypeParams::DirectTcpIp { .. }) {
        match bind_direct_tcpip_listener(config, channel_health).await {
          Ok(listener) => {
            let session = session.clone();
            let config = config.clone();
            let cancel = cancel.clone();
            channel_tasks.spawn(async move {
              (
                index,
                run_direct_tcpip_listener(session, &config, cancel, listener).await,
              )
            });
          }
          Err(error) if !error.is_retryable() => {
            error!(channel = %config.name, error = ?error, "Local forward disabled");
            set_health(
              channel_health,
              ChannelHealth::Failed {
                error: error.to_string(),
              },
            );
          }
          Err(error) => return Err(error),
        }
      }
    }

    if active_reverse_forwards == 0 && channel_tasks.is_empty() {
      return Err(AppError::SshChannel(
        "No channels could be started on the shared SSH session".into(),
      ));
    }

    let mut session_check = tokio::time::interval(Duration::from_millis(500));
    session_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
      tokio::select! {
        _ = cancel.cancelled() => {
          channel_tasks.shutdown().await;
          drop(jumps);
          return Ok(());
        }
        _ = session_check.tick() => {
          if session.is_closed() {
            channel_tasks.shutdown().await;
            return established_session_outcome(AppError::SshConnection(
              "SSH session closed".into(),
            ));
          }
        }
        task = channel_tasks.join_next(), if !channel_tasks.is_empty() => {
          match task {
            Some(Ok((_, Ok(())))) if cancel.is_cancelled() => {}
            Some(Ok((index, Ok(())))) => {
              set_health(
                &health[index],
                ChannelHealth::Failed {
                  error: "Channel task stopped unexpectedly".into(),
                },
              );
            }
            Some(Ok((index, Err(error)))) if !error.is_retryable() => {
              error!(channel = %configs[index].name, error = ?error, "Channel disabled");
              set_health(
                &health[index],
                ChannelHealth::Failed {
                  error: error.to_string(),
                },
              );
            }
            Some(Ok((_, Err(error)))) => {
              channel_tasks.shutdown().await;
              return established_session_outcome(error);
            }
            Some(Err(error)) => {
              channel_tasks.shutdown().await;
              return Err(AppError::SshChannel(format!("Channel task failed: {}", error)));
            }
            None => {}
          }
          if active_reverse_forwards == 0 && channel_tasks.is_empty() {
            return Err(AppError::SshChannel(
              "No channels remain active on the shared SSH session".into(),
            ));
          }
        }
      }
    }
  }
}

/// Briefly lock and overwrite the health cell. Logs and swallows poison: the
/// reconnect loop should never abort just because a previous panic poisoned
/// the mutex — losing one badge update is preferable to taking the whole
/// channel down.
fn set_health(cell: &Arc<StdMutex<ChannelHealth>>, next: ChannelHealth) {
  match cell.lock() {
    Ok(mut g) => *g = next,
    Err(poisoned) => *poisoned.into_inner() = next,
  }
}

fn set_all_health(cells: &[Arc<StdMutex<ChannelHealth>>], next: ChannelHealth) {
  for cell in cells {
    set_health(cell, next.clone());
  }
}

fn set_failed_unless_already(cell: &Arc<StdMutex<ChannelHealth>>, error: &str) {
  let set_if_needed = |health: &mut ChannelHealth| {
    if !matches!(health, ChannelHealth::Failed { .. }) {
      *health = ChannelHealth::Failed {
        error: error.to_string(),
      };
    }
  };
  match cell.lock() {
    Ok(mut health) => set_if_needed(&mut health),
    Err(poisoned) => set_if_needed(&mut poisoned.into_inner()),
  }
}

fn channel_status(config: &ChannelConfig, health: &Arc<StdMutex<ChannelHealth>>) -> ChannelStatus {
  let (direction, local, remote) = match &config.params {
    ChannelTypeParams::DirectTcpIp {
      listen_host,
      local_port,
      dest_host,
      dest_port,
    } => (
      Direction::LocalToRemote,
      format!("{}:{}", listen_host, local_port),
      format!("{}:{}", dest_host, dest_port),
    ),
    ChannelTypeParams::ForwardedTcpIp {
      remote_bind_host,
      remote_bind_port,
      local_connect_host,
      local_connect_port,
    } => (
      Direction::RemoteToLocal,
      format!("{}:{}", local_connect_host, local_connect_port),
      format!("{}:{}", remote_bind_host, remote_bind_port),
    ),
  };
  let health = health
    .lock()
    .map(|value| value.clone())
    .unwrap_or(ChannelHealth::Stopped);
  ChannelStatus {
    name: config.name.clone(),
    direction,
    local,
    remote,
    health,
  }
}

fn exhausted_retry_backoff(initial_delay: Duration) -> ExponentialBackoff {
  ExponentialBuilder::default()
    .with_min_delay(initial_delay)
    .with_max_delay(EXHAUSTED_RETRY_MAX_DELAY)
    .with_max_times(usize::MAX)
    .with_jitter()
    .build()
}

fn jittered_retry_delay(delay: Duration) -> Duration {
  ExponentialBuilder::default()
    .with_min_delay(delay)
    .with_max_delay(delay)
    .with_max_times(1)
    .with_jitter()
    .build()
    .next()
    .expect("single retry delay")
}

/// `Ok` tells the outer manager to reset the finite retry cycle only after a
/// working session later ends with a transient transport error.
fn established_session_outcome(error: AppError) -> Result<()> {
  if error.is_retryable() {
    Ok(())
  } else {
    Err(error)
  }
}

/// Register one remote forward on an already-authenticated shared session.
async fn register_forwarded_tcpip(
  session: &mut client::Handle<ClientHandler>,
  config: &ChannelConfig,
  reverse_routes: ReverseRoutes,
  health: Arc<StdMutex<ChannelHealth>>,
) -> Result<()> {
  let ChannelTypeParams::ForwardedTcpIp {
    remote_bind_host,
    remote_bind_port,
    local_connect_host,
    local_connect_port,
  } = &config.params
  else {
    return Err(AppError::SshChannel(
      "register_forwarded_tcpip expects ForwardedTcpIp params".to_string(),
    ));
  };

  info!(
      channel = %config.name,
      remote_bind = %format!("{}:{}", remote_bind_host, remote_bind_port),
      "Requesting remote port forward (tcpip-forward)"
  );

  let bound_port = session
    .tcpip_forward(remote_bind_host.as_str(), *remote_bind_port as u32)
    .await
    .map_err(|error| match error {
      russh::Error::Disconnect
      | russh::Error::HUP
      | russh::Error::ConnectionTimeout
      | russh::Error::KeepaliveTimeout
      | russh::Error::InactivityTimeout
      | russh::Error::SendError
      | russh::Error::IO(_) => AppError::SshConnection(format!("tcpip-forward failed: {}", error)),
      _ => AppError::SshChannel(format!("tcpip-forward failed: {}", error)),
    })?;

  let actual_port = if *remote_bind_port == 0 {
    bound_port
  } else {
    *remote_bind_port as u32
  };
  let destination = ReverseDestination {
    channel_name: config.name.clone(),
    local_addr: format!("{}:{}", local_connect_host, local_connect_port),
  };
  match reverse_routes.write() {
    Ok(mut routes) => {
      routes.insert(actual_port, destination);
    }
    Err(poisoned) => {
      poisoned.into_inner().insert(actual_port, destination);
    }
  }

  info!(
      channel = %config.name,
      remote = %format!("{}:{}", remote_bind_host, actual_port),
      local = %format!("{}:{}", local_connect_host, local_connect_port),
      "Remote forward active (incoming connections will be bridged to local)"
  );
  set_health(&health, ChannelHealth::Connected);
  Ok(())
}

/// Match OpenSSH's behavior by preferring host-key algorithms already trusted
/// for this host. Strict verification still happens in the client handler.
fn make_client_config(host: &str, port: u16) -> Arc<russh::client::Config> {
  let mut config = russh::client::Config {
    keepalive_interval: Some(Duration::from_secs(15)),
    keepalive_max: 3,
    ..Default::default()
  };

  if let Ok(keys) = russh_keys::known_host_keys(host, port) {
    let known_algorithms: Vec<&str> = keys.iter().map(|(_, key)| key.name()).collect();
    prioritize_known_host_key_algorithms(config.preferred.key.to_mut(), &known_algorithms);
  }

  Arc::new(config)
}

fn prioritize_known_host_key_algorithms(
  algorithms: &mut [russh_keys::key::Name],
  known_algorithms: &[&str],
) {
  algorithms.sort_by_key(|algorithm| !known_algorithms.contains(&algorithm.0));
}

fn map_ssh_connect_error(context: String, error: russh::Error) -> AppError {
  match &error {
    russh::Error::UnknownKey | russh::Error::KeyChanged { .. } => {
      AppError::SshHostKey(format!("{}: {}", context, error))
    }
    _ => AppError::SshConnection(format!("{}: {}", context, error)),
  }
}

/// Establish the SSH session to the channel's target, optionally walking a
/// ProxyJump chain on the way. Returns the chain of jump handles (which must
/// be kept alive for the lifetime of the target session) plus the
/// authenticated terminal session.
///
/// Topology: first hop is dialed via plain TCP; each subsequent hop and the
/// terminal are reached by opening a `direct-tcpip` channel on the previous
/// session and laying a new SSH handshake on top of it.
async fn connect_via_chain<H>(
  config: &ChannelConfig,
  terminal_handler: H,
) -> Result<(Vec<client::Handle<JumpClientHandler>>, client::Handle<H>)>
where
  H: client::Handler<Error = russh::Error> + Send + 'static,
{
  let mut hops: Vec<client::Handle<JumpClientHandler>> =
    Vec::with_capacity(config.proxy_jumps.len());

  for (i, hop) in config.proxy_jumps.iter().enumerate() {
    let russh_cfg = make_client_config(&hop.host, hop.port);
    let handler = JumpClientHandler {
      alias: hop.alias.clone(),
      host: hop.host.clone(),
      port: hop.port,
    };

    let mut session = if i == 0 {
      info!(
          channel = %config.name,
          hop = %hop.alias,
          host = %hop.host,
          port = hop.port,
          "Connecting to ProxyJump (first hop, TCP)"
      );
      russh::client::connect(russh_cfg.clone(), (hop.host.as_str(), hop.port), handler)
        .await
        .map_err(|error| {
          map_ssh_connect_error(
            format!(
              "Failed to connect to ProxyJump '{}' ({}:{})",
              hop.alias, hop.host, hop.port
            ),
            error,
          )
        })?
    } else {
      let prev_alias = config.proxy_jumps[i - 1].alias.clone();
      info!(
          channel = %config.name,
          hop = %hop.alias,
          via = %prev_alias,
          "Tunneling to next ProxyJump"
      );
      let prev = hops.last().expect("hops non-empty after first iteration");
      let channel = prev
        .channel_open_direct_tcpip(hop.host.as_str(), hop.port as u32, "127.0.0.1", 0u32)
        .await
        .map_err(|e| {
          AppError::SshConnection(format!(
            "Failed to open jump channel through '{}' to '{}': {:?}",
            prev_alias, hop.alias, e
          ))
        })?;
      let stream = channel.into_stream();
      russh::client::connect_stream(russh_cfg.clone(), stream, handler)
        .await
        .map_err(|error| {
          map_ssh_connect_error(
            format!(
              "SSH handshake with ProxyJump '{}' (via '{}') failed",
              hop.alias, prev_alias
            ),
            error,
          )
        })?
    };

    authenticate_jump_publickey(&mut session, &hop.alias, &hop.username, &hop.key_path).await?;
    hops.push(session);
  }

  let russh_cfg = make_client_config(&config.host, config.port);
  let mut terminal: client::Handle<H> = if hops.is_empty() {
    russh::client::connect(
      russh_cfg.clone(),
      (config.host.as_str(), config.port),
      terminal_handler,
    )
    .await
    .map_err(|error| map_ssh_connect_error("Failed to connect".into(), error))?
  } else {
    let prev_alias = config
      .proxy_jumps
      .last()
      .expect("hops non-empty")
      .alias
      .clone();
    info!(
        channel = %config.name,
        host = %config.host,
        port = config.port,
        via = %prev_alias,
        "Tunneling to target via final ProxyJump"
    );
    let prev = hops.last().expect("hops non-empty");
    let channel = prev
      .channel_open_direct_tcpip(config.host.as_str(), config.port as u32, "127.0.0.1", 0u32)
      .await
      .map_err(|e| {
        AppError::SshConnection(format!(
          "Failed to open target channel through ProxyJump '{}': {:?}",
          prev_alias, e
        ))
      })?;
    let stream = channel.into_stream();
    russh::client::connect_stream(russh_cfg, stream, terminal_handler)
      .await
      .map_err(|error| map_ssh_connect_error("SSH handshake with target failed".into(), error))?
  };

  info!(channel = %config.name, "SSH connection established, authenticating");
  authenticate_terminal(&mut terminal, &config.username, &config.auth).await?;
  info!(channel = %config.name, "Authentication successful");

  Ok((hops, terminal))
}

/// Authenticate a jump hop using publickey only. The key must be unencrypted —
/// daemons can't prompt for a passphrase.
async fn authenticate_jump_publickey(
  session: &mut client::Handle<JumpClientHandler>,
  alias: &str,
  username: &str,
  key_path: &Path,
) -> Result<()> {
  let key = load_jump_key(key_path, alias).await?;
  let authenticated = session
    .authenticate_publickey(username, Arc::new(key))
    .await
    .map_err(|e| {
      AppError::SshAuthentication(format!(
        "Public-key auth failed at ProxyJump '{}': {}",
        alias, e
      ))
    })?;
  ensure_auth_succeeded(
    authenticated,
    format!("Public-key auth rejected at ProxyJump '{}'", alias),
  )?;
  Ok(())
}

/// Authenticate the terminal session using whichever AuthConfig the channel was
/// resolved with. Mirrors the original direct-connect path's auth logic.
async fn authenticate_terminal<H>(
  session: &mut client::Handle<H>,
  username: &str,
  auth: &AuthConfig,
) -> Result<()>
where
  H: client::Handler + Send,
{
  match auth {
    AuthConfig::Password { password } => {
      let authenticated = session
        .authenticate_password(username, password)
        .await
        .map_err(|e| {
          AppError::SshAuthentication(format!("Password authentication failed: {}", e))
        })?;
      ensure_auth_succeeded(authenticated, "Password authentication rejected")?;
    }
    AuthConfig::Key {
      key_path,
      passphrase,
    } => {
      let key = load_secret_key(key_path, passphrase.as_deref()).await?;
      let authenticated = session
        .authenticate_publickey(username, Arc::new(key))
        .await
        .map_err(|e| AppError::SshAuthentication(format!("Key authentication failed: {}", e)))?;
      ensure_auth_succeeded(authenticated, "Key authentication rejected")?;
    }
  }
  Ok(())
}

fn ensure_auth_succeeded(
  message_is_success: bool,
  rejected_message: impl Into<String>,
) -> Result<()> {
  if message_is_success {
    Ok(())
  } else {
    Err(AppError::SshAuthentication(rejected_message.into()))
  }
}

/// Load an unencrypted private key for a jump hop. Surfaces a tailored error
/// when the key is passphrase-protected so the user knows daemon-mode can't
/// prompt and points them at the fix.
async fn load_jump_key(key_path: &Path, alias: &str) -> Result<KeyPair> {
  let key_path = key_path.to_path_buf();
  let alias = alias.to_string();
  tokio::task::spawn_blocking(move || {
    let data = std::fs::read_to_string(&key_path).map_err(AppError::Io)?;
    match russh_keys::decode_secret_key(&data, None) {
      Ok(k) => Ok(k),
      Err(russh_keys::Error::KeyIsEncrypted) => Err(AppError::SshAuthentication(format!(
        "ProxyJump alias '{}' uses encrypted IdentityFile '{}'. This tool does \
         not prompt for passphrases on jump hosts — decrypt the key or point \
         IdentityFile at an unencrypted one.",
        alias,
        key_path.display()
      ))),
      Err(e) => Err(AppError::SshAuthentication(format!(
        "Failed to decode ProxyJump key for '{}' ({}): {}",
        alias,
        key_path.display(),
        e
      ))),
    }
  })
  .await
  .map_err(|e| AppError::SshAuthentication(format!("Task join error: {}", e)))?
}

/// Load SSH private key
async fn load_secret_key(key_path: &Path, passphrase: Option<&str>) -> Result<KeyPair> {
  let key_path = key_path.to_path_buf();
  let passphrase = passphrase.map(|s| s.to_string());

  tokio::task::spawn_blocking(move || {
    let key_data = std::fs::read_to_string(&key_path).map_err(AppError::Io)?;

    let key_result = if let Some(passphrase) = passphrase {
      russh_keys::decode_secret_key(&key_data, Some(&passphrase))
    } else {
      russh_keys::decode_secret_key(&key_data, None)
    };

    key_result.map_err(|e| AppError::SshAuthentication(format!("Failed to decode key: {}", e)))
  })
  .await
  .map_err(|e| AppError::SshAuthentication(format!("Task join error: {}", e)))?
}

async fn bind_direct_tcpip_listener(
  config: &ChannelConfig,
  health: &Arc<StdMutex<ChannelHealth>>,
) -> Result<TcpListener> {
  let ChannelTypeParams::DirectTcpIp {
    listen_host,
    local_port,
    ..
  } = &config.params
  else {
    return Err(AppError::SshChannel(
      "bind_direct_tcpip_listener expects DirectTcpIp params".to_string(),
    ));
  };

  let listen_addr = format!("{}:{}", listen_host, local_port);
  let listener = TcpListener::bind(&listen_addr).await.map_err(|e| {
    AppError::SshChannel(format!(
      "Failed to bind {}: {}. Try another port or run as admin for port < 1024.",
      listen_addr, e
    ))
  })?;

  info!(
      channel = %config.name,
      listen = %listen_addr,
      "Local listener started, accepting connections"
  );
  // Listener bound → ready to relay. Flip before the accept loop so `status`
  // sees Connected immediately, not only after the first client connects.
  set_health(health, ChannelHealth::Connected);
  Ok(listener)
}

/// Run a bound local TCP listener and forward connections via direct-tcpip channels.
async fn run_direct_tcpip_listener(
  session: Arc<client::Handle<ClientHandler>>,
  config: &ChannelConfig,
  cancel: CancellationToken,
  listener: TcpListener,
) -> Result<()> {
  let ChannelTypeParams::DirectTcpIp {
    dest_host,
    dest_port,
    ..
  } = &config.params
  else {
    return Err(AppError::SshChannel(
      "run_direct_tcpip_listener expects DirectTcpIp params".to_string(),
    ));
  };

  loop {
    tokio::select! {
        _ = cancel.cancelled() => {
            info!(channel = %config.name, "Listener cancelled");
            return Ok(());
        }
        accept_result = listener.accept() => {
            let (mut stream, peer_addr) = match accept_result {
                Ok(x) => x,
                Err(e) => {
                    error!(channel = %config.name, error = ?e, "Accept failed");
                    continue;
                }
            };
            let channel_name = config.name.clone();
            let dest_host = dest_host.clone();
            let dest_port = *dest_port;
            match session.channel_open_direct_tcpip(
                &dest_host,
                dest_port as u32,
                "127.0.0.1",
                0u32,
            ).await {
                Ok(channel) => {
                    debug!(
                        channel = %channel_name,
                        peer = %peer_addr,
                        dest = %format!("{}:{}", dest_host, dest_port),
                        "Direct TCP/IP channel opened for connection"
                    );
                    let mut channel_stream = channel.into_stream();
                    tokio::spawn(async move {
                        if let Err(e) =
                            tokio::io::copy_bidirectional(&mut stream, &mut channel_stream).await
                        {
                            debug!(channel = %channel_name, error = ?e, "Relay ended");
                        }
                    });
                }
                Err(e @ Error::ChannelOpenFailure(_)) => {
                    error!(
                        channel = %channel_name,
                        peer = %peer_addr,
                        error = ?e,
                        "Channel open refused by server (connection alive)"
                    );
                }
                Err(e) => {
                    error!(
                        channel = %channel_name,
                        error = ?e,
                        "SSH session dead detected via channel_open, triggering reconnection"
                    );
                    return Err(AppError::SshConnection(
                        format!("SSH session dead: {}", e)
                    ));
                }
            }
        }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn auth_success_helper_accepts_true() {
    assert!(ensure_auth_succeeded(true, "password authentication rejected").is_ok());
  }

  #[test]
  fn auth_success_helper_rejects_false() {
    let err = ensure_auth_succeeded(false, "password authentication rejected").unwrap_err();
    let msg = err.to_string();
    assert!(
      msg.contains("password authentication rejected"),
      "expected rejected auth message, got: {msg}"
    );
  }

  #[test]
  fn known_host_key_algorithms_are_negotiated_first() {
    let mut algorithms = russh::Preferred::default().key.into_owned();

    prioritize_known_host_key_algorithms(&mut algorithms, &["ecdsa-sha2-nistp256"]);

    assert_eq!(algorithms[0], russh_keys::key::ECDSA_SHA2_NISTP256);
    assert_eq!(algorithms[1], russh_keys::key::ED25519);
  }

  #[test]
  fn exhausted_retry_backoff_reaches_sixty_seconds_with_jitter() {
    let mut backoff = exhausted_retry_backoff(Duration::from_secs(1));

    for base_secs in [1, 2, 4, 8, 16, 32, 60, 60] {
      let delay = backoff.next().expect("unlimited retry delay");
      assert!(delay >= Duration::from_secs(base_secs));
      assert!(delay < Duration::from_secs(base_secs + 1));
    }
  }

  #[test]
  fn only_transient_errors_reset_an_established_session_retry_cycle() {
    assert!(
      established_session_outcome(AppError::SshConnection("reset".into())).is_ok(),
      "a dropped working session should start a fresh retry cycle"
    );
    assert!(
      established_session_outcome(AppError::SshChannel("listener failed".into())).is_err(),
      "permanent errors must not be swallowed after a session was established"
    );
  }
}
