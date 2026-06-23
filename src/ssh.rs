use crate::config::{AuthConfig, ChannelConfig, ChannelTypeParams, Direction, ReconnectionConfig};
use crate::error::{AppError, Result};
use crate::service::{ChannelHealth, ChannelStatus};
use backon::{ExponentialBuilder, Retryable};
use russh::*;
use russh_keys::key::KeyPair;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// SSH client handler for ProxyJump hops.
///
/// Differs from [`ClientHandler`] in `check_server_key`: jump hops are verified
/// against `~/.ssh/known_hosts` strictly — an unknown host or a changed key
/// causes the handshake to fail. There is no trust-on-first-use because this
/// is a non-interactive daemon.
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
    match russh_keys::check_known_hosts(&self.host, self.port, server_public_key) {
      Ok(true) => Ok(true),
      Ok(false) => {
        error!(
          alias = %self.alias,
          host = %self.host,
          port = self.port,
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
struct ClientHandler;

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
  type Error = russh::Error;

  async fn check_server_key(
    &mut self,
    _server_public_key: &russh_keys::key::PublicKey,
  ) -> std::result::Result<bool, Self::Error> {
    Ok(true) // Accept any server key (in production, verify this)
  }
}

/// Handler for forwarded-tcpip (remote forwarding, ssh -R style).
/// When the server opens a forwarded-tcpip channel, connect to local_host:local_port and bridge.
#[derive(Clone)]
struct ReverseForwardHandler {
  channel_name: String,
  local_host: String,
  local_port: u16,
}

#[async_trait::async_trait]
impl client::Handler for ReverseForwardHandler {
  type Error = russh::Error;

  async fn check_server_key(
    &mut self,
    _server_public_key: &russh_keys::key::PublicKey,
  ) -> std::result::Result<bool, Self::Error> {
    Ok(true)
  }

  async fn server_channel_open_forwarded_tcpip(
    &mut self,
    channel: russh::Channel<russh::client::Msg>,
    _connected_address: &str,
    _connected_port: u32,
    _originator_address: &str,
    _originator_port: u32,
    _session: &mut russh::client::Session,
  ) -> std::result::Result<(), Self::Error> {
    let local_addr = format!("{}:{}", self.local_host, self.local_port);
    let channel_name = self.channel_name.clone();

    match TcpStream::connect(&local_addr).await {
      Ok(mut stream) => {
        let mut channel_stream = channel.into_stream();
        tokio::spawn(async move {
          if let Err(e) = tokio::io::copy_bidirectional(&mut stream, &mut channel_stream).await {
            debug!(channel = %channel_name, error = ?e, "Forwarded-tcpip relay ended");
          }
        });
      }
      Err(e) => {
        error!(
            channel = %channel_name,
            local = %local_addr,
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
  config: ChannelConfig,
  reconnection_config: ReconnectionConfig,
  shutdown_tx: Option<mpsc::Sender<()>>,
  cancellation_token: Option<CancellationToken>,
  /// Live channel health, shared with the spawned connect loop. Uses
  /// `std::sync::Mutex` because we only hold the lock for state writes
  /// (never across `.await`), and `backon::Retry::notify` takes a sync
  /// closure that needs to do the same.
  health: Arc<StdMutex<ChannelHealth>>,
}

impl SshManager {
  /// Create a new SSH manager
  pub fn new(config: ChannelConfig, reconnection_config: ReconnectionConfig) -> Self {
    Self {
      config,
      reconnection_config,
      shutdown_tx: None,
      cancellation_token: None,
      health: Arc::new(StdMutex::new(ChannelHealth::Stopped)),
    }
  }

  /// Snapshot for the `status` command: channel topology plus current health.
  pub fn snapshot(&self) -> ChannelStatus {
    let (direction, local, remote) = match &self.config.params {
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
    let health = self
      .health
      .lock()
      .map(|h| h.clone())
      .unwrap_or(ChannelHealth::Stopped);
    ChannelStatus {
      name: self.config.name.clone(),
      direction,
      local,
      remote,
      health,
    }
  }

  /// Start managing the SSH connection and channel
  pub async fn start(&mut self) -> Result<()> {
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let cancel = CancellationToken::new();
    self.cancellation_token = Some(cancel.clone());
    self.shutdown_tx = Some(shutdown_tx);

    let config = self.config.clone();
    let reconnection_config = self.reconnection_config.clone();
    let health = self.health.clone();

    set_health(&health, ChannelHealth::Connecting { attempt: 1 });

    tokio::spawn(async move {
      loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!(channel = %config.name, "Shutting down SSH manager");
                break;
            }
            _ = cancel.cancelled() => break,
            result = Self::connect_and_manage_channel(&config, &reconnection_config, cancel.clone(), health.clone()) => {
                match result {
                    Ok(_) => {
                        warn!(channel = %config.name, "Connection closed unexpectedly");
                    }
                    Err(e) => {
                        error!(channel = %config.name, error = ?e, "Connection error");
                        set_health(&health, ChannelHealth::Failed { error: e.to_string() });
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        // Outer loop restarts the connect cycle even after `max_retries` is
        // exhausted (project design — see top-level comment on ChannelHealth).
        // Reset the badge so the user sees a fresh attempt counter.
        set_health(&health, ChannelHealth::Connecting { attempt: 1 });
      }
      set_health(&health, ChannelHealth::Stopped);
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
    set_health(&self.health, ChannelHealth::Stopped);
    Ok(())
  }

  /// Connect and manage SSH channel with reconnection logic
  async fn connect_and_manage_channel(
    config: &ChannelConfig,
    reconnection_config: &ReconnectionConfig,
    cancel: CancellationToken,
    health: Arc<StdMutex<ChannelHealth>>,
  ) -> Result<()> {
    // Build retry policy
    let builder = if reconnection_config.use_exponential_backoff {
      let mut builder = ExponentialBuilder::default()
        .with_min_delay(Duration::from_secs(reconnection_config.initial_delay_secs))
        .with_max_delay(Duration::from_secs(reconnection_config.max_delay_secs));

      if reconnection_config.max_retries > 0 {
        builder = builder.with_max_times(reconnection_config.max_retries as usize);
      }

      builder
    } else {
      // For fixed interval, use exponential with same min/max delay
      let mut builder = ExponentialBuilder::default()
        .with_min_delay(Duration::from_secs(reconnection_config.initial_delay_secs))
        .with_max_delay(Duration::from_secs(reconnection_config.initial_delay_secs));

      if reconnection_config.max_retries > 0 {
        builder = builder.with_max_times(reconnection_config.max_retries as usize);
      }

      builder
    };

    // Retry connection with backoff. Each attempt increments `attempt_counter`;
    // backon's `notify` fires between attempts so we can flip to Reconnecting
    // with the failure cause. The success path flips to Connected from inside
    // the run_* helpers, before they block on the session.
    let attempt_counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let health_for_attempt = health.clone();
    let health_for_notify = health.clone();
    let attempt_for_notify = attempt_counter.clone();

    (|| {
      let n = attempt_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
      set_health(
        &health_for_attempt,
        ChannelHealth::Connecting { attempt: n },
      );
      let health = health.clone();
      let cancel = cancel.clone();
      async move { Self::establish_connection(config, cancel, health).await }
    })
    .retry(&builder)
    .notify(move |err, dur| {
      let attempt = attempt_for_notify.load(std::sync::atomic::Ordering::Relaxed);
      warn!(
          channel = %config.name,
          attempt,
          backoff_ms = dur.as_millis() as u64,
          error = %err,
          "Connect attempt failed, will retry"
      );
      set_health(
        &health_for_notify,
        ChannelHealth::Reconnecting {
          attempt,
          last_error: err.to_string(),
        },
      );
    })
    .await
    .map_err(|e| AppError::SshConnection(format!("Failed to establish connection: {}", e)))
  }

  /// Establish SSH connection and open channel
  async fn establish_connection(
    config: &ChannelConfig,
    cancel: CancellationToken,
    health: Arc<StdMutex<ChannelHealth>>,
  ) -> Result<()> {
    if config.proxy_jumps.is_empty() {
      info!(
          channel = %config.name,
          host = %config.host,
          port = config.port,
          "Establishing SSH connection"
      );
    } else {
      let chain: Vec<&str> = config
        .proxy_jumps
        .iter()
        .map(|h| h.alias.as_str())
        .collect();
      info!(
          channel = %config.name,
          host = %config.host,
          port = config.port,
          via = %chain.join(" -> "),
          "Establishing SSH connection through ProxyJump chain"
      );
    }

    match &config.params {
      ChannelTypeParams::ForwardedTcpIp {
        local_connect_host,
        local_connect_port,
        ..
      } => {
        let handler = ReverseForwardHandler {
          channel_name: config.name.clone(),
          local_host: local_connect_host.clone(),
          local_port: *local_connect_port,
        };
        // _jumps is held here for the lifetime of the connection — dropping any
        // hop closes its `direct-tcpip` channel and cascades the failure to the
        // terminal session, which the reconnect loop then retries.
        let (_jumps, mut session) = connect_via_chain(config, handler).await?;
        drive_forwarded_tcpip(&mut session, config, cancel, health).await
      }
      ChannelTypeParams::DirectTcpIp { .. } => {
        let (_jumps, mut session) = connect_via_chain(config, ClientHandler).await?;
        info!(channel = %config.name, "Opening channel");
        run_direct_tcpip_listener(&mut session, config, cancel, health).await
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

/// Drive remote port forwarding (ssh -R style) on an already-authenticated session:
/// ask server to bind a port, then keep the session alive until cancellation or
/// the session ends.
async fn drive_forwarded_tcpip(
  session: &mut client::Handle<ReverseForwardHandler>,
  config: &ChannelConfig,
  cancel: CancellationToken,
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
      "drive_forwarded_tcpip expects ForwardedTcpIp params".to_string(),
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
    .map_err(|e| AppError::SshChannel(format!("tcpip-forward failed: {}", e)))?;

  let actual_port = if bound_port == 0 {
    *remote_bind_port
  } else {
    bound_port as u16
  };

  info!(
      channel = %config.name,
      remote = %format!("{}:{}", remote_bind_host, actual_port),
      local = %format!("{}:{}", local_connect_host, local_connect_port),
      "Remote forward active (incoming connections will be bridged to local)"
  );
  // tcpip-forward registered → the channel is serving. Flip the badge here,
  // before we block on the session future, so `status` sees Connected even
  // when traffic is idle.
  set_health(&health, ChannelHealth::Connected);

  tokio::select! {
      _ = cancel.cancelled() => {
          info!(channel = %config.name, "Forward cancelled");
          Ok(())
      }
      result = &mut *session => {
          result.map_err(|e| AppError::SshConnection(format!("Session ended: {}", e)))
      }
  }
}

/// Construct the shared russh client config (same keepalive policy for every hop).
fn make_client_config() -> Arc<russh::client::Config> {
  Arc::new(russh::client::Config {
    keepalive_interval: Some(Duration::from_secs(15)),
    keepalive_max: 3,
    ..Default::default()
  })
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
  H: client::Handler + Send + 'static,
{
  let russh_cfg = make_client_config();
  let mut hops: Vec<client::Handle<JumpClientHandler>> =
    Vec::with_capacity(config.proxy_jumps.len());

  for (i, hop) in config.proxy_jumps.iter().enumerate() {
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
        .map_err(|e| {
          AppError::SshConnection(format!(
            "Failed to connect to ProxyJump '{}' ({}:{}): {:?}",
            hop.alias, hop.host, hop.port, e
          ))
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
        .map_err(|e| {
          AppError::SshConnection(format!(
            "SSH handshake with ProxyJump '{}' (via '{}') failed: {:?}",
            hop.alias, prev_alias, e
          ))
        })?
    };

    authenticate_jump_publickey(&mut session, &hop.alias, &hop.username, &hop.key_path).await?;
    hops.push(session);
  }

  let mut terminal: client::Handle<H> = if hops.is_empty() {
    russh::client::connect(
      russh_cfg.clone(),
      (config.host.as_str(), config.port),
      terminal_handler,
    )
    .await
    .map_err(|e| AppError::SshConnection(format!("Failed to connect: {:?}", e)))?
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
      .map_err(|e| AppError::SshConnection(format!("SSH handshake with target failed: {:?}", e)))?
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

/// Run local TCP listener and forward each connection via a new direct-tcpip channel.
async fn run_direct_tcpip_listener(
  session: &mut client::Handle<ClientHandler>,
  config: &ChannelConfig,
  cancel: CancellationToken,
  health: Arc<StdMutex<ChannelHealth>>,
) -> Result<()> {
  let ChannelTypeParams::DirectTcpIp {
    listen_host,
    local_port,
    dest_host,
    dest_port,
  } = &config.params
  else {
    return Err(AppError::SshChannel(
      "run_direct_tcpip_listener expects DirectTcpIp params".to_string(),
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
  set_health(&health, ChannelHealth::Connected);

  loop {
    tokio::select! {
        _ = cancel.cancelled() => {
            info!(channel = %config.name, "Listener cancelled");
            return Ok(());
        }
        result = &mut *session => {
            let reason = result.map_err(|e| e.to_string())
                .err()
                .unwrap_or_else(|| "connection closed".to_string());
            warn!(
                channel = %config.name,
                reason = %reason,
                "SSH session ended, triggering reconnection"
            );
            return Err(AppError::SshConnection(
                format!("SSH session ended: {}", reason)
            ));
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
}
