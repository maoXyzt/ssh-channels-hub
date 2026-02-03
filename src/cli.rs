use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// SSH Channels Hub - Manage SSH connections and channels
#[derive(Parser)]
#[command(name = "ssh-channels-hub")]
#[command(about = "A CLI application to create and manage SSH channels", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Configuration file path
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Enable debug logging
    #[arg(short, long, global = true)]
    pub debug: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start the service
    Start {
        /// Run as daemon in background (spawns detached child process)
        #[arg(short = 'D', long)]
        daemon: bool,
    },
    /// Stop the service
    Stop,
    /// Restart the service
    Restart,
    /// Show service status
    Status,
    /// Validate configuration file
    Validate {
        /// Configuration file to validate
        config: Option<PathBuf>,
    },
    /// Generate configuration from SSH config file
    Generate {
        /// SSH config file path (default: ~/.ssh/config)
        #[arg(short, long)]
        ssh_config: Option<PathBuf>,
        /// Output TOML config file path
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Test if channels are actually working by connecting to local ports
    Test {
        /// Configuration file path
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
}
