use crate::error::{AppError, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::PathBuf;

/// SSH host definition (previously channel definition)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostConfig {
    /// Host name/identifier (used by channels to reference)
    pub name: String,
    /// Remote host address
    pub host: String,
    /// SSH port (defaults to 22)
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// SSH username
    pub username: String,
    /// Authentication method
    pub auth: AuthConfig,
}

fn default_ssh_port() -> u16 {
    22
}

/// Port forwarding configuration (local:dest format)
#[derive(Debug, Clone)]
pub struct PortForward {
    /// Local port to bind (required)
    pub local_port: Option<u16>,
    /// Destination port (required)
    pub dest_port: u16,
}

impl PortForward {
    /// Parse port forward string in format "local:dest"
    /// Both local and dest ports are required (e.g., "80:3923")
    fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err(AppError::Config(format!(
                "Invalid port format '{}'. Expected format: 'local:dest' (e.g., '80:3923')",
                s
            )));
        }

        if parts[0].is_empty() {
            return Err(AppError::Config(format!(
                "Invalid port format '{}'. Local port cannot be empty. Expected format: 'local:dest' (e.g., '80:3923')",
                s
            )));
        }

        if parts[1].is_empty() {
            return Err(AppError::Config(format!(
                "Invalid port format '{}'. Destination port cannot be empty. Expected format: 'local:dest' (e.g., '80:3923')",
                s
            )));
        }

        let local_port = parts[0]
            .parse::<u16>()
            .map_err(|e| AppError::Config(format!("Invalid local port '{}': {}", parts[0], e)))?;

        let dest_port = parts[1].parse::<u16>().map_err(|e| {
            AppError::Config(format!("Invalid destination port '{}': {}", parts[1], e))
        })?;

        Ok(PortForward {
            local_port: Some(local_port),
            dest_port,
        })
    }
}

impl<'de> Deserialize<'de> for PortForward {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        PortForward::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl Serialize for PortForward {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let local = self.local_port.expect("local_port must be set");
        let s = format!("{}:{}", local, self.dest_port);
        serializer.serialize_str(&s)
    }
}

/// Channel definition referencing a host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Channel name/identifier
    pub name: String,
    /// Host reference (must match hosts.name)
    pub hostname: String,
    /// Channel type: "direct-tcpip" (local forward, like ssh -L) or "forwarded-tcpip" (remote forward, like ssh -R).
    /// Default: "direct-tcpip"
    #[serde(default)]
    pub channel_type: Option<String>,
    /// Port forwarding configuration.
    /// For direct-tcpip: "local:dest" (local listen port : remote dest port). Example: "80:3923"
    /// For forwarded-tcpip: "remote:local" (remote bind port : local connect port). Example: "8022:80"
    pub ports: PortForward,
    /// For direct-tcpip: destination host on remote (defaults to 127.0.0.1).
    /// For forwarded-tcpip: local host to connect to (defaults to 127.0.0.1).
    #[serde(default = "default_destination_host")]
    pub dest_host: String,
    /// Local listen address for direct-tcpip (defaults to 127.0.0.1).
    /// Use "0.0.0.0" to accept connections from any interface.
    /// Ignored for forwarded-tcpip.
    #[serde(default = "default_listen_host")]
    pub listen_host: String,
}

fn default_listen_host() -> String {
    "127.0.0.1".to_string()
}

fn default_destination_host() -> String {
    "127.0.0.1".to_string()
}

/// SSH channel configuration (runtime)
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// Channel name/identifier
    pub name: String,
    /// Remote host address
    pub host: String,
    /// SSH port (defaults to 22)
    pub port: u16,
    /// SSH username
    pub username: String,
    /// Authentication method
    pub auth: AuthConfig,
    /// Channel type string for logging and status display (e.g. "direct-tcpip", "forwarded-tcpip")
    #[allow(dead_code)]
    pub channel_type: String,
    /// Parameters specific to the channel type; semantics are explicit per variant
    pub params: ChannelTypeParams,
}

/// Parameters for each channel type. Makes intent explicit and type-safe.
#[derive(Debug, Clone)]
pub enum ChannelTypeParams {
    /// Local port forwarding (ssh -L): listen locally, forward to remote dest.
    DirectTcpIp {
        listen_host: String,
        local_port: u16,
        dest_host: String,
        dest_port: u16,
    },
    /// Remote port forwarding (ssh -R): server binds port, we connect to local and bridge.
    ForwardedTcpIp {
        remote_bind_port: u16,
        local_connect_host: String,
        local_connect_port: u16,
    },
    /// Session channel (e.g. shell or single command).
    Session { command: Option<String> },
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
}

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// SSH hosts definition (replaces channels)
    pub hosts: Vec<HostConfig>,
    /// Channels referencing hosts
    #[serde(default)]
    pub channels: Vec<ConnectionConfig>,
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

    /// Default config file candidates (first existing wins; if none exist, first is used).
    /// Order: current directory `configs.toml`, then platform config dir `config.toml`.
    pub fn default_path_candidates() -> Vec<PathBuf> {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut candidates = vec![current_dir.join("configs.toml")];
        if let Some(mut path) = dirs::config_dir() {
            path.push("ssh-channels-hub");
            path.push("config.toml");
            candidates.push(path);
        }
        candidates
    }

    /// Get default configuration file path: first candidate that exists, or first candidate.
    pub fn default_path() -> PathBuf {
        for path in Self::default_path_candidates() {
            if path.exists() {
                return path;
            }
        }
        Self::default_path_candidates()
            .into_iter()
            .next()
            .unwrap_or_else(|| PathBuf::from("configs.toml"))
    }

    /// Generate configuration from SSH config entries
    pub fn from_ssh_config_entries(entries: Vec<crate::ssh_config::SshConfigEntry>) -> Self {
        let mut hosts = Vec::new();

        for entry in entries.into_iter() {
            // Skip entries without required fields
            let hostname = match entry.hostname {
                Some(h) => h,
                None => continue,
            };
            let username = match entry.user {
                Some(u) => u,
                None => continue,
            };

            // Determine authentication method
            let auth = if let Some(key_path) = entry.identity_file {
                AuthConfig::Key {
                    key_path,
                    passphrase: None, // Passphrase not available from SSH config
                }
            } else {
                // If no identity file, we'll use password auth as placeholder
                // User will need to fill in the password manually
                AuthConfig::Password {
                    password: "CHANGE_ME".to_string(),
                }
            };

            let host_cfg = HostConfig {
                name: entry.host.clone(),
                host: hostname,
                port: entry.port.unwrap_or(22), // Use port from SSH config or default to 22
                username,
                auth,
            };

            hosts.push(host_cfg);
        }

        Self {
            hosts,
            channels: Vec::new(), // Generate command doesn't create channels
            reconnection: ReconnectionConfig::default(),
        }
    }

    /// Build runtime channel configs by combining hosts and channels
    pub fn build_channels(&self) -> Result<Vec<ChannelConfig>> {
        let mut channels = Vec::new();

        for conn in &self.channels {
            let host_cfg = self
                .hosts
                .iter()
                .find(|h| h.name == conn.hostname)
                .ok_or_else(|| {
                    AppError::Config(format!(
                        "Channel '{}' references unknown host '{}'",
                        conn.name, conn.hostname
                    ))
                })?;

            let channel_type = conn
                .channel_type
                .as_deref()
                .unwrap_or("direct-tcpip")
                .to_string();

            let params = match channel_type.as_str() {
                "forwarded-tcpip" => {
                    let local_connect_port = conn.ports.local_port.ok_or_else(|| {
                        AppError::Config(format!(
                            "Channel '{}': forwarded-tcpip requires ports local:remote (e.g. 80:8022)",
                            conn.name
                        ))
                    })?;
                    ChannelTypeParams::ForwardedTcpIp {
                        remote_bind_port: conn.ports.dest_port,
                        local_connect_host: conn.dest_host.clone(),
                        local_connect_port,
                    }
                }
                "session" => ChannelTypeParams::Session { command: None },
                _ => {
                    let local_port = conn.ports.local_port.ok_or_else(|| {
                        AppError::Config(format!(
                            "Channel '{}': direct-tcpip requires ports local:remote (e.g. 8080:80)",
                            conn.name
                        ))
                    })?;
                    ChannelTypeParams::DirectTcpIp {
                        listen_host: conn.listen_host.clone(),
                        local_port,
                        dest_host: conn.dest_host.clone(),
                        dest_port: conn.ports.dest_port,
                    }
                }
            };

            channels.push(ChannelConfig {
                name: conn.name.clone(),
                host: host_cfg.host.clone(),
                port: host_cfg.port,
                username: host_cfg.username.clone(),
                auth: host_cfg.auth.clone(),
                channel_type,
                params,
            });
        }

        Ok(channels)
    }

    /// Save configuration to a TOML file
    pub fn to_file(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;

        // Add comments before each [[hosts]] entry
        let content_with_comments = self.add_host_comments(&content);

        std::fs::write(path.as_ref(), content_with_comments)
            .map_err(|e| AppError::Config(format!("Failed to write config file: {}", e)))?;

        Ok(())
    }

    /// Add comments before each [[hosts]] entry
    fn add_host_comments(&self, content: &str) -> String {
        let mut result = String::new();
        let lines: Vec<&str> = content.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];

            // Check if this is a [[hosts]] line
            if line.trim() == "[[hosts]]" {
                // Find the corresponding host config to get its name
                if let Some(host_idx) = self.find_host_index(&lines, i) {
                    let host = &self.hosts[host_idx];

                    // Check if there's already a blank line before this entry
                    let has_blank_before = i > 0 && lines[i - 1].trim().is_empty();

                    // Add a blank line if there isn't one already
                    if !has_blank_before && !result.trim().is_empty() {
                        result.push('\n');
                    }

                    // Add comment with host information
                    result.push_str(&format!("# Host: {} ({})\n", host.name, host.host));
                }
            }

            result.push_str(line);
            result.push('\n');
            i += 1;
        }

        result
    }

    /// Find which host index corresponds to a [[hosts]] line at a given position
    fn find_host_index(&self, lines: &[&str], start_pos: usize) -> Option<usize> {
        // Count how many [[hosts]] entries appear before this position
        let host_count = lines
            .iter()
            .take(start_pos)
            .filter(|line| line.trim() == "[[hosts]]")
            .count();

        if host_count < self.hosts.len() {
            Some(host_count)
        } else {
            None
        }
    }
}
