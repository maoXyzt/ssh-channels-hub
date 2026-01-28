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
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
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
}
