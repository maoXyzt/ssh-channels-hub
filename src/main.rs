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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
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
            handle_stop(config_path).await?;
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

    // Start IPC listener so "status" command can query this process
    let cancel = CancellationToken::new();
    let port = start_ipc_listener(&config_path, Arc::clone(&service_manager), cancel.clone())
        .await
        .context("Failed to start IPC listener for status queries")?;
    write_pid_file(&pid_file_path(&config_path)).context("Write PID file")?;
    info!(
        "Status query listener on 127.0.0.1:{} (status command will connect here)",
        port
    );

    if foreground {
        info!("Service running in foreground. Press Ctrl+C to stop.");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = cancel.cancelled() => {}
        }

        info!("Shutdown signal received, stopping service...");
    } else {
        info!("Daemon mode not yet implemented, running in foreground");
        info!("Service running. Press Ctrl+C to stop.");

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = cancel.cancelled() => {}
        }

        info!("Shutdown signal received, stopping service...");
    }

    cancel.cancel();
    let _ = remove_run_files(&config_path);
    service_manager
        .stop()
        .await
        .context("Failed to stop service")?;

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

fn remove_run_files(config_path: &Path) -> AnyhowResult<()> {
    for path in [pid_file_path(config_path), port_file_path(config_path)] {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Serialize ServiceStatus to TOML (one-way protocol: server sends, client reads).
fn status_to_toml(status: &service::ServiceStatus) -> String {
    let state_str = match &status.state {
        ServiceState::Running => "Running",
        ServiceState::Stopped => "Stopped",
        ServiceState::Starting => "Starting",
        ServiceState::Stopping => "Stopping",
        ServiceState::Error(_) => "Error",
    };
    format!(
        "state = \"{}\"\nactive_channels = {}\ntotal_channels = {}",
        state_str, status.active_channels, status.total_channels
    )
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
                            tokio::spawn(async move {
                                if let Err(e) = handle_ipc_connection(stream, manager).await {
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

/// Handle one status-query connection: read optional request, send current status as TOML, close.
async fn handle_ipc_connection(
    mut stream: TcpStream,
    service_manager: Arc<ServiceManager>,
) -> AnyhowResult<()> {
    // Optional: read and ignore request (e.g. "status\n")
    let _ = stream.read(&mut [0u8; 64]).await;
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

#[derive(serde::Deserialize)]
struct StatusResponse {
    state: String,
    active_channels: usize,
    total_channels: usize,
}

fn parse_status_toml(s: &str) -> AnyhowResult<service::ServiceStatus> {
    let r: StatusResponse = toml::from_str(s).context("Parse status TOML")?;
    let state = match r.state.as_str() {
        "Running" => ServiceState::Running,
        "Stopped" => ServiceState::Stopped,
        "Starting" => ServiceState::Starting,
        "Stopping" => ServiceState::Stopping,
        "Error" => ServiceState::Error(String::new()),
        _ => return Err(anyhow::anyhow!("Unknown state: {}", r.state)),
    };
    Ok(service::ServiceStatus {
        state,
        active_channels: r.active_channels,
        total_channels: r.total_channels,
    })
}

/// Handle stop command
async fn handle_stop(config_path: PathBuf) -> AnyhowResult<()> {
    info!("Stop command received");
    remove_run_files(&config_path).context("Remove run files")?;
    info!("Run files (.pid, .port) removed if present");
    info!("Note: If the service is running in another terminal, stop it with Ctrl+C there.");
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

/// Handle status command: connect to main process via IPC to get live status.
async fn handle_status(config_path: PathBuf) -> AnyhowResult<()> {
    // Try IPC first: connect to running main process
    match query_status_via_ipc(&config_path).await {
        Ok(status) => {
            println!("Service Status:");
            println!("  State: {:?}", status.state);
            println!(
                "  Active Channels: {}/{}",
                status.active_channels, status.total_channels
            );
            return Ok(());
        }
        Err(_) => {}
    }

    // No running process (IPC file missing or connection refused): show Stopped with config totals
    if !config_path.exists() {
        println!("✗ Service not configured (config file not found)");
        return Ok(());
    }

    match AppConfig::from_file(&config_path) {
        Ok(config) => {
            let total = config.channels.len();
            println!("Service Status:");
            println!("  State: {:?}", ServiceState::Stopped);
            println!("  Active Channels: 0/{}", total);
            println!(
                "  Note: Service is not running. Start with: ssh-channels-hub start --foreground"
            );
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
