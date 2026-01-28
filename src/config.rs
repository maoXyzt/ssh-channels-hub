use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// SSH channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Channel name/identifier
    pub name: String,
    /// Remote host address
    pub host: String,
    /// SSH port (default: 22)
    #[serde(default = "default_port")]
    pub port: u16,
    /// SSH username
    pub username: String,
    /// Authentication method
    pub auth: AuthConfig,
    /// Channel type (e.g., "session", "direct-tcpip")
    #[serde(default = "default_channel_type")]
    pub channel_type: String,
    /// Additional channel parameters
    #[serde(default)]
    pub params: ChannelParams,
}

fn default_port() -> u16 {
    22
}

fn default_channel_type() -> String {
    "session".to_string()
}

/// Authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthConfig {
    /// Password authentication
    #[serde(rename = "password")]
    Password { password: String },
    /// Private key authentication
    #[serde(rename = "key")]
    Key {
        /// Path to private key file
        key_path: PathBuf,
        /// Optional passphrase for the key
        passphrase: Option<String>,
    },
    /// Agent authentication (use SSH agent)
    #[serde(rename = "agent")]
    Agent,
}

/// Channel parameters
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelParams {
    /// For direct-tcpip: destination host
    pub destination_host: Option<String>,
    /// For direct-tcpip: destination port
    pub destination_port: Option<u16>,
    /// For session: command to execute
    pub command: Option<String>,
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// List of SSH channels to manage
    pub channels: Vec<ChannelConfig>,
    /// Reconnection settings
    #[serde(default)]
    pub reconnection: ReconnectionConfig,
}

/// Reconnection configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconnectionConfig {
    /// Maximum retry attempts (0 = unlimited)
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Initial delay in seconds before retry
    #[serde(default = "default_initial_delay")]
    pub initial_delay_secs: u64,
    /// Maximum delay in seconds between retries
    #[serde(default = "default_max_delay")]
    pub max_delay_secs: u64,
    /// Use exponential backoff (true) or fixed interval (false)
    #[serde(default = "default_use_exponential")]
    pub use_exponential_backoff: bool,
}

fn default_max_retries() -> u32 {
    0 // Unlimited by default
}

fn default_initial_delay() -> u64 {
    1
}

fn default_max_delay() -> u64 {
    30
}

fn default_use_exponential() -> bool {
    true
}

impl Default for ReconnectionConfig {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            initial_delay_secs: default_initial_delay(),
            max_delay_secs: default_max_delay(),
            use_exponential_backoff: default_use_exponential(),
        }
    }
}

impl AppConfig {
    /// Load configuration from a TOML file
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| AppError::Config(format!("Failed to read config file: {}", e)))?;

        let config: AppConfig = toml::from_str(&content)
            .map_err(|e| AppError::Config(format!("Failed to parse config: {}", e)))?;

        Ok(config)
    }

    /// Get default configuration file path
    pub fn default_path() -> PathBuf {
        if let Some(mut path) = dirs::config_dir() {
            path.push("ssh-channels-hub");
            path.push("config.toml");
            path
        } else {
            // Fallback to current directory if config dir is not available
            PathBuf::from("config.toml")
        }
    }
}
