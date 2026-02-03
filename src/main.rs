mod cli;
mod config;
mod error;
mod port_check;
mod service;
mod ssh;
mod ssh_config;

use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
use clap::Parser;
use cli::{Cli, Commands};
use config::AppConfig;
use port_check::test_port_connection;
use service::{ServiceManager, ServiceState};
use ssh_config::{default_ssh_config_path, parse_ssh_config};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> AnyhowResult<()> {
    let cli = Cli::parse();

    // Initialize logging
    init_logging(cli.debug)?;

    // Determine config path
    let config_path = cli.config.clone().unwrap_or_else(AppConfig::default_path);

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
async fn handle_start(config_path: std::path::PathBuf, foreground: bool) -> AnyhowResult<()> {
    info!("Loading configuration from: {}", config_path.display());

    let config = AppConfig::from_file(&config_path).context("Failed to load configuration")?;

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

    // In a real implementation, you would:
    // 1. Read PID file to find running service
    // 2. Connect to the service instance
    // 3. Call service_manager.restart()
    // For now, we'll load config and create a new service manager
    info!("Loading configuration from: {}", config_path.display());

    let config = AppConfig::from_file(&config_path).context("Failed to load configuration")?;

    let service_manager = Arc::new(ServiceManager::new(config));

    // Use the restart method
    service_manager
        .restart()
        .await
        .context("Failed to restart service")?;

    info!("Service restarted successfully");
    Ok(())
}

/// Handle status command
async fn handle_status(config_path: std::path::PathBuf) -> AnyhowResult<()> {
    // In a real implementation, you would:
    // 1. Read PID file to find running service
    // 2. Connect to the service instance via IPC
    // 3. Call service_manager.status()
    // For now, we'll try to load config and show status
    // Note: This will only work if the service is running in the same process

    if !config_path.exists() {
        println!("✗ Service not configured (config file not found)");
        return Ok(());
    }

    match AppConfig::from_file(&config_path) {
        Ok(config) => {
            let service_manager = Arc::new(ServiceManager::new(config));
            let status = service_manager.status().await;

            println!("Service Status:");
            println!("  State: {:?}", status.state);
            println!(
                "  Active Channels: {}/{}",
                status.active_channels, status.total_channels
            );

            // Note: This shows the status of a newly created manager, not the running service
            // Full functionality requires IPC or PID file management
            if status.active_channels == 0 && status.state == ServiceState::Stopped {
                println!("  Note: Service appears to be stopped");
                println!("  Note: Full status requires connecting to running service instance");
            }
        }
        Err(e) => {
            println!("✗ Failed to load configuration: {}", e);
            return Err(anyhow::anyhow!("Failed to load config: {}", e));
        }
    }

    Ok(())
}

/// Handle validate command
async fn handle_validate(config_path: Option<std::path::PathBuf>) -> AnyhowResult<()> {
    let path = config_path
        .ok_or_else(|| anyhow::anyhow!("Configuration file path required for validation"))?;

    info!("Validating configuration file: {}", path.display());

    match AppConfig::from_file(&path) {
        Ok(config) => {
            println!("✓ Configuration is valid");
            println!("  Hosts configured: {}", config.hosts.len());
            for host in &config.hosts {
                println!("    - {} ({})", host.name, host.host);
            }
            println!("  Channels configured: {}", config.channels.len());
            for conn in &config.channels {
                let local = conn.ports.local_port.expect("local_port must be set");
                let port_info = format!("{}:{}", local, conn.ports.dest_port);
                println!("    - {} -> {}:{}", conn.name, conn.dest_host, port_info);
            }
            Ok(())
        }
        Err(e) => {
            error!("✗ Configuration validation failed: {}", e);
            Err(anyhow::anyhow!("Invalid configuration: {}", e))
        }
    }
}

/// Handle generate command
async fn handle_generate(
    ssh_config: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
) -> AnyhowResult<()> {
    let ssh_config_path = ssh_config.unwrap_or_else(default_ssh_config_path);

    info!("Reading SSH config from: {}", ssh_config_path.display());

    let entries = parse_ssh_config(&ssh_config_path).context("Failed to parse SSH config file")?;

    if entries.is_empty() {
        println!("⚠ No valid SSH config entries found");
        return Ok(());
    }

    info!("Found {} SSH config entries", entries.len());

    let app_config = AppConfig::from_ssh_config_entries(entries);

    let output_path = output.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("configs.toml")
    });

    info!("Generating configuration to: {}", output_path.display());

    app_config
        .to_file(&output_path)
        .context("Failed to write configuration file")?;

    println!("✓ Configuration generated successfully");
    println!("  Output file: {}", output_path.display());
    println!("  Hosts generated: {}", app_config.hosts.len());
    for host in &app_config.hosts {
        println!("    - {} ({})", host.name, host.host);
    }

    // Warn about password placeholders
    let password_hosts: Vec<_> = app_config
        .hosts
        .iter()
        .filter(|h| matches!(h.auth, config::AuthConfig::Password { .. }))
        .collect();

    if !password_hosts.is_empty() {
        println!(
            "\n⚠ Warning: {} host(s) use password authentication with placeholder 'CHANGE_ME'",
            password_hosts.len()
        );
        println!("  Please update the password in the generated config file.");
    }

    println!(
        "\n💡 Note: You need to manually add [[channels]] sections to define port forwarding."
    );

    Ok(())
}

/// Handle test command - verify channels are working
async fn handle_test(config_path: std::path::PathBuf) -> AnyhowResult<()> {
    info!("Loading configuration from: {}", config_path.display());

    let config = AppConfig::from_file(&config_path).context("Failed to load configuration")?;

    if config.channels.is_empty() {
        println!("No channels configured");
        return Ok(());
    }

    println!("Testing {} channel(s)...\n", config.channels.len());

    let mut all_passed = true;

    for conn in &config.channels {
        let local_port = conn.ports.local_port.expect("local_port must be set");
        let dest_port = conn.ports.dest_port;
        let dest_host = &conn.dest_host;

        print!(
            "Testing channel '{}' (local:{} -> {}:{})... ",
            conn.name, local_port, dest_host, dest_port
        );

        // Test connection to local port
        match test_port_connection("127.0.0.1", local_port).await {
            Ok(true) => {
                println!("✓ Connected");
            }
            Ok(false) => {
                println!("✗ Failed to connect");
                all_passed = false;
            }
            Err(e) => {
                println!("✗ Error: {}", e);
                all_passed = false;
            }
        }
    }

    println!();

    if all_passed {
        println!("✓ All channels are working correctly!");
        Ok(())
    } else {
        println!("✗ Some channels failed the connection test");
        println!("\nTroubleshooting tips:");
        println!(
            "1. Make sure the service is running: cargo run start -c {}",
            config_path.display()
        );
        println!("2. Check if ports are listening: netstat -an | grep LISTEN");
        println!("3. Verify SSH connection is established (check logs with --debug)");
        println!("4. Ensure remote service is accessible from the SSH server");
        Err(anyhow::anyhow!("Some channels failed the connection test"))
    }
}
