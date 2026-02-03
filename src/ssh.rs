use crate::config::{AuthConfig, ChannelConfig, ReconnectionConfig};
use crate::error::{AppError, Result};
use backon::{ExponentialBuilder, Retryable};
use russh::*;
use russh_keys::key::KeyPair;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// SSH client handler
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

/// SSH connection manager
pub struct SshManager {
    config: ChannelConfig,
    reconnection_config: ReconnectionConfig,
    shutdown_tx: Option<mpsc::Sender<()>>,
    cancellation_token: Option<CancellationToken>,
}

impl SshManager {
    /// Create a new SSH manager
    pub fn new(config: ChannelConfig, reconnection_config: ReconnectionConfig) -> Self {
        Self {
            config,
            reconnection_config,
            shutdown_tx: None,
            cancellation_token: None,
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

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!(channel = %config.name, "Shutting down SSH manager");
                        break;
                    }
                    _ = cancel.cancelled() => break,
                    result = Self::connect_and_manage_channel(&config, &reconnection_config, cancel.clone()) => {
                        match result {
                            Ok(_) => {
                                warn!(channel = %config.name, "Connection closed unexpectedly");
                            }
                            Err(e) => {
                                error!(channel = %config.name, error = ?e, "Connection error");
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
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
        Ok(())
    }

    /// Connect and manage SSH channel with reconnection logic
    async fn connect_and_manage_channel(
        config: &ChannelConfig,
        reconnection_config: &ReconnectionConfig,
        cancel: CancellationToken,
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

        // Retry connection with backoff
        (|| async { Self::establish_connection(config, cancel.clone()).await })
            .retry(&builder)
            .await
            .map_err(|e| AppError::SshConnection(format!("Failed to establish connection: {}", e)))
    }

    /// Establish SSH connection and open channel
    async fn establish_connection(config: &ChannelConfig, cancel: CancellationToken) -> Result<()> {
        info!(
            channel = %config.name,
            host = %config.host,
            port = config.port,
            "Establishing SSH connection"
        );

        let config_builder = russh::client::Config::default();

        let config_arc = Arc::new(config_builder);
        let handler = ClientHandler;

        let mut session =
            russh::client::connect(config_arc, (config.host.as_str(), config.port), handler)
                .await
                .map_err(|e| AppError::SshConnection(format!("Failed to connect: {}", e)))?;

        info!(channel = %config.name, "SSH connection established, authenticating");

        // Authenticate
        match &config.auth {
            AuthConfig::Password { password } => {
                session
                    .authenticate_password(&config.username, password)
                    .await
                    .map_err(|e| {
                        AppError::SshAuthentication(format!(
                            "Password authentication failed: {}",
                            e
                        ))
                    })?;
            }
            AuthConfig::Key {
                key_path,
                passphrase,
            } => {
                let key = load_secret_key(key_path, passphrase.as_deref()).await?;

                session
                    .authenticate_publickey(&config.username, Arc::new(key))
                    .await
                    .map_err(|e| {
                        AppError::SshAuthentication(format!("Key authentication failed: {}", e))
                    })?;
            }
        }

        info!(channel = %config.name, "Authentication successful, opening channel");

        match config.channel_type.as_str() {
            "session" => {
                open_session_channel(&mut session, config).await?;
                info!(channel = %config.name, "Channel opened successfully");
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
            "direct-tcpip" => {
                return run_direct_tcpip_listener(&mut session, config, cancel).await;
            }
            _ => {
                return Err(AppError::SshChannel(format!(
                    "Unsupported channel type: {}",
                    config.channel_type
                )));
            }
        }
    }
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

/// Open a session channel
async fn open_session_channel(
    session: &mut client::Handle<ClientHandler>,
    config: &ChannelConfig,
) -> Result<()> {
    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| AppError::SshChannel(format!("Failed to open session channel: {}", e)))?;

    // If a command is specified, execute it
    if let Some(command) = &config.params.command {
        channel
            .exec(true, command.as_str())
            .await
            .map_err(|e| AppError::SshChannel(format!("Failed to execute command: {}", e)))?;
    } else {
        // Open a shell - request PTY first
        channel
            .request_pty(false, "xterm", 80, 24, 0, 0, &[])
            .await
            .map_err(|e| AppError::SshChannel(format!("Failed to request PTY: {}", e)))?;

        // For session channels without a command, we keep it open
        // The shell will be opened when data is sent
        info!(channel = %config.name, "Session channel ready");
    }

    // Spawn task to handle channel data
    let channel_id = channel.id();
    tokio::spawn({
        let mut channel = channel;
        async move {
            loop {
                match channel.wait().await {
                    Some(msg) => {
                        debug!(channel_id = %channel_id, message = ?msg, "Channel message");
                        // Handle channel messages
                    }
                    None => {
                        warn!(channel_id = %channel_id, "Channel closed");
                        break;
                    }
                }
            }
        }
    });

    Ok(())
}

/// Run local TCP listener and forward each connection via a new direct-tcpip channel.
async fn run_direct_tcpip_listener(
    session: &mut client::Handle<ClientHandler>,
    config: &ChannelConfig,
    cancel: CancellationToken,
) -> Result<()> {
    let local_port = config.params.local_port.ok_or_else(|| {
        AppError::SshChannel(
            "local_port required for direct-tcpip (ports format: local:dest, e.g. 80:3923)"
                .to_string(),
        )
    })?;

    let listen_host = config.params.listen_host.as_deref().unwrap_or("127.0.0.1");
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
                let dest_host = config
                    .params
                    .destination_host
                    .as_deref()
                    .unwrap_or("127.0.0.1")
                    .to_string();
                let dest_port = match config.params.destination_port {
                    Some(p) => p,
                    None => {
                        error!(channel = %config.name, "destination_port not set");
                        continue;
                    }
                };
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
                    Err(e) => {
                        error!(
                            channel = %channel_name,
                            error = ?e,
                            "Failed to open direct-tcpip channel for new connection"
                        );
                    }
                }
            }
        }
    }
}
