mod cli;
mod config;
mod error;
mod service;
mod ssh;

use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
use clap::Parser;
use cli::{Cli, Commands};
use config::AppConfig;
use service::ServiceManager;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> AnyhowResult<()> {
    let cli = Cli::parse();

    // Initialize logging
    init_logging(cli.debug)?;

    // Determine config path
    let config_path = cli.config.clone().unwrap_or_else(|| {
        AppConfig::default_path()
    });

    // Handle commands
    match cli.command {
        Commands::Start { foreground } => {
            handle_start(config_path, foreground).await?;
        }
        Commands::Stop => {
            handle_stop().await?;
        }
        Commands::Restart => {
            handle_restart(config_path).await?;
        }
        Commands::Status => {
            handle_status().await?;
        }
        Commands::Validate { config } => {
            let path = config.or(Some(config_path));
            handle_validate(path).await?;
        }
    }

    Ok(())
}

/// Initialize logging subsystem
fn init_logging(debug: bool) -> AnyhowResult<()> {
    let filter = if debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    Ok(())
}

/// Handle start command
async fn handle_start(config_path: std::path::PathBuf, foreground: bool) -> AnyhowResult<()> {
    info!("Loading configuration from: {}", config_path.display());

    let config = AppConfig::from_file(&config_path)
        .context("Failed to load configuration")?;

    info!("Configuration loaded successfully");

    let service_manager = Arc::new(ServiceManager::new(config));

    // Start the service
    service_manager
        .start()
        .await
        .context("Failed to start service")?;

    if foreground {
        info!("Service running in foreground. Press Ctrl+C to stop.");

        // Wait for shutdown signal
        tokio::signal::ctrl_c()
            .await
            .context("Failed to listen for shutdown signal")?;

        info!("Shutdown signal received, stopping service...");
        service_manager
            .stop()
            .await
            .context("Failed to stop service")?;
    } else {
        // In a real daemon implementation, you would:
        // 1. Fork the process
        // 2. Write PID file
        // 3. Detach from terminal
        // For now, we'll run in foreground with a note
        info!("Daemon mode not yet implemented, running in foreground");
        info!("Service running. Press Ctrl+C to stop.");

        tokio::signal::ctrl_c()
            .await
            .context("Failed to listen for shutdown signal")?;

        info!("Shutdown signal received, stopping service...");
        service_manager
            .stop()
            .await
            .context("Failed to stop service")?;
    }

    Ok(())
}

/// Handle stop command
async fn handle_stop() -> AnyhowResult<()> {
    // In a real implementation, you would:
    // 1. Read PID file
    // 2. Send signal to the process
    // For now, this is a placeholder
    info!("Stop command received");
    info!("Note: Full daemon stop functionality requires PID file management");
    Ok(())
}

/// Handle restart command
async fn handle_restart(config_path: std::path::PathBuf) -> AnyhowResult<()> {
    info!("Restart command received");
    handle_stop().await?;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    handle_start(config_path, false).await?;
    Ok(())
}

/// Handle status command
async fn handle_status() -> AnyhowResult<()> {
    // In a real implementation, you would:
    // 1. Read PID file
    // 2. Check if process is running
    // 3. Connect to service to get status
    // For now, this is a placeholder
    info!("Status command received");
    info!("Note: Full status functionality requires IPC or PID file management");
    Ok(())
}

/// Handle validate command
async fn handle_validate(config_path: Option<std::path::PathBuf>) -> AnyhowResult<()> {
    let path = config_path.ok_or_else(|| {
        anyhow::anyhow!("Configuration file path required for validation")
    })?;

    info!("Validating configuration file: {}", path.display());

    match AppConfig::from_file(&path) {
        Ok(config) => {
            println!("✓ Configuration is valid");
            println!("  Channels configured: {}", config.channels.len());
            for channel in &config.channels {
                println!("    - {} ({}:{})", channel.name, channel.host, channel.port);
            }
            Ok(())
        }
        Err(e) => {
            error!("✗ Configuration validation failed: {}", e);
            Err(anyhow::anyhow!("Invalid configuration: {}", e))
        }
    }
}
