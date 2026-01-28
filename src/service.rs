use crate::config::AppConfig;
use crate::error::{AppError, Result};
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

        let mut managers = Vec::new();
        let mut errors = Vec::new();

        for channel_config in &self.config.channels {
            let mut manager =
                SshManager::new(channel_config.clone(), self.config.reconnection.clone());

            match manager.start().await {
                Ok(_) => {
                    info!(channel = %channel_config.name, "Started SSH manager");
                    managers.push(manager);
                }
                Err(e) => {
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

    /// Restart the service
    pub async fn restart(&self) -> Result<()> {
        info!("Restarting SSH Channels Hub service");
        self.stop().await?;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        self.start().await
    }

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
