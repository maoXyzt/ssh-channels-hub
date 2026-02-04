use crate::config::{AppConfig, ChannelTypeParams};
use crate::error::{AppError, Result};
use crate::port_check::check_ports;
use crate::ssh::SshManager;
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
        let ports_to_check: Vec<u16> = self
            .config
            .channels
            .iter()
            .filter_map(|conn| conn.ports.local_port)
            .collect();

        if !ports_to_check.is_empty() {
            info!(
                "Checking port availability for {} port(s)",
                ports_to_check.len()
            );
            match check_ports(&ports_to_check).await {
                Ok(occupied) => {
                    if !occupied.is_empty() {
                        let error_msg = format!(
                            "Port(s) already in use: {}. Please stop the application using these ports or change the configuration.",
                            occupied.iter().map(|p: &u16| p.to_string()).collect::<Vec<_>>().join(", ")
                        );
                        error!(ports = ?occupied, "Port check failed");
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
            let mut manager =
                SshManager::new(channel_config.clone(), self.config.reconnection.clone());

            match manager.start().await {
                Ok(_) => {
                    match &channel_config.params {
                        ChannelTypeParams::ForwardedTcpIp {
                            remote_bind_port,
                            local_connect_host,
                            local_connect_port,
                        } => {
                            let local_dest =
                                format!("{}:{}", local_connect_host, local_connect_port);
                            println!(
                                "✓ Channel '{}' started: remote:{} -> local {} ({}@{})",
                                channel_config.name,
                                remote_bind_port,
                                local_dest,
                                channel_config.username,
                                channel_config.host
                            );
                        }
                        ChannelTypeParams::DirectTcpIp {
                            local_port,
                            dest_host,
                            dest_port,
                            ..
                        } => {
                            let dest_info = format!("{}:{}", dest_host, dest_port);
                            println!(
                                "✓ Channel '{}' started: local:{} -> {} -> {}@{}",
                                channel_config.name,
                                local_port,
                                dest_info,
                                channel_config.username,
                                channel_config.host
                            );
                        }
                        ChannelTypeParams::Session { .. } => {
                            println!(
                                "✓ Channel '{}' started (session) ({}@{})",
                                channel_config.name, channel_config.username, channel_config.host
                            );
                        }
                    }

                    info!(channel = %channel_config.name, "Started SSH manager");
                    managers.push(manager);
                }
                Err(e) => {
                    println!("✗ Channel '{}' failed to start: {}", channel_config.name, e);
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

        if errors.is_empty() {
            *state = ServiceState::Running;
            println!(
                "\n✓ Service started successfully with {} active channel(s)",
                managers_guard.len()
            );
            info!("Service started successfully");
            Ok(())
        } else if managers_guard.is_empty() {
            *state = ServiceState::Error(format!("All channels failed: {}", errors.join(", ")));
            Err(AppError::Service(format!(
                "Failed to start any channels: {}",
                errors.join(", ")
            )))
        } else {
            *state = ServiceState::Running;
            println!(
                "\n⚠ Service started with {} active channel(s), {} failed",
                managers_guard.len(),
                errors.len()
            );
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

    /// Get service status
    pub async fn status(&self) -> ServiceStatus {
        let state = self.state.lock().await.clone();
        let managers = self.managers.lock().await;
        let channel_count = managers.len();
        let total_channels = self.config.channels.len();

        ServiceStatus {
            state,
            active_channels: channel_count,
            total_channels,
        }
    }
}

/// Service status information
#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub active_channels: usize,
    pub total_channels: usize,
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "State: {:?}, Channels: {}/{}",
            self.state, self.active_channels, self.total_channels
        )
    }
}
