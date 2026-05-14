use crate::error::{AppError, Result};
use crate::ssh_config;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::path::PathBuf;

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

    let dest_port = parts[1]
      .parse::<u16>()
      .map_err(|e| AppError::Config(format!("Invalid destination port '{}': {}", parts[1], e)))?;

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

/// Channel definition referencing an SSH config Host alias
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
  /// Channel name/identifier
  pub name: String,
  /// SSH config Host alias (must match a `Host <alias>` block in ~/.ssh/config)
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

/// SSH channel configuration (runtime — built by combining configs.toml + ~/.ssh/config)
#[derive(Debug, Clone)]
pub struct ChannelConfig {
  /// Channel name/identifier
  pub name: String,
  /// Remote host address (resolved from SSH config HostName)
  pub host: String,
  /// SSH port (resolved from SSH config Port, default 22)
  pub port: u16,
  /// SSH username (resolved from SSH config User)
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

/// Authentication configuration (runtime — used by SSH layer)
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

/// Per-host credential override.
///
/// SSH config (`~/.ssh/config`) cannot store secrets — Password / IdentityFile passphrase
/// live here. Both fields are optional; provide whichever applies for the host.
///
/// Resolution rules (see `AppConfig::build_channels`):
/// - If `password` is set, password auth is used regardless of any IdentityFile in SSH config.
/// - Otherwise the host's IdentityFile is required, and `passphrase` is attached to it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthOverride {
  #[serde(default)]
  pub password: Option<String>,
  #[serde(default)]
  pub passphrase: Option<String>,
}

/// Application configuration
///
/// Host info (HostName / User / Port / IdentityFile) lives in `~/.ssh/config`.
/// This file defines:
/// - which channels to bring up (`[[channels]]`, referencing SSH config aliases)
/// - per-host credentials SSH config can't hold (`[auth.<alias>]`)
/// - reconnection policy
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
  /// Optional override for the SSH config file path. None → `~/.ssh/config`.
  #[serde(default)]
  pub ssh_config: Option<PathBuf>,
  /// Channels referencing SSH config Host aliases
  #[serde(default)]
  pub channels: Vec<ConnectionConfig>,
  /// Per-host credential overrides keyed by SSH config Host alias.
  /// Only required when SSH config alone can't authenticate (password, key with passphrase).
  #[serde(default)]
  pub auth: HashMap<String, AuthOverride>,
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

  /// Resolved SSH config path (override from configs.toml or platform default).
  pub fn ssh_config_path(&self) -> PathBuf {
    self
      .ssh_config
      .clone()
      .unwrap_or_else(ssh_config::default_ssh_config_path)
  }

  /// Build runtime channel configs by resolving each `[[channels]]` entry against
  /// the SSH config and applying any `[auth.<alias>]` overrides.
  pub fn build_channels(&self) -> Result<Vec<ChannelConfig>> {
    let ssh_config_path = self.ssh_config_path();
    let entries = ssh_config::parse_ssh_config(&ssh_config_path).map_err(|e| {
      AppError::Config(format!(
        "Failed to read SSH config at {}: {}",
        ssh_config_path.display(),
        e
      ))
    })?;

    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      entries.iter().map(|e| (e.host.as_str(), e)).collect();

    let mut channels = Vec::new();
    for conn in &self.channels {
      let entry = by_alias
        .get(conn.hostname.as_str())
        .copied()
        .ok_or_else(|| {
          AppError::Config(format!(
            "Channel '{}' references host alias '{}', but no `Host {}` block exists in {}",
            conn.name,
            conn.hostname,
            conn.hostname,
            ssh_config_path.display()
          ))
        })?;

      let host = entry.hostname.clone().ok_or_else(|| {
        AppError::Config(format!(
          "SSH config Host '{}' is missing `HostName`",
          conn.hostname
        ))
      })?;

      let username = entry.user.clone().ok_or_else(|| {
        AppError::Config(format!(
          "SSH config Host '{}' is missing `User`",
          conn.hostname
        ))
      })?;

      let port = entry.port.unwrap_or(22);

      let override_ = self.auth.get(&conn.hostname);
      let auth = resolve_auth(&conn.hostname, entry, override_)?;

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
        "direct-tcpip" => {
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
        unknown => {
          return Err(AppError::Config(format!(
            "Channel '{}': unknown channel_type '{}', expected 'direct-tcpip', 'forwarded-tcpip', or 'session'",
            conn.name, unknown
          )));
        }
      };

      channels.push(ChannelConfig {
        name: conn.name.clone(),
        host,
        port,
        username,
        auth,
        channel_type,
        params,
      });
    }

    Ok(channels)
  }

  /// Render a commented-out configs.toml scaffold from SSH config entries.
  /// Used by the `generate` subcommand.
  pub fn generate_scaffold(entries: &[ssh_config::SshConfigEntry]) -> String {
    let mut out = String::new();
    out.push_str("# SSH Channels Hub configuration\n");
    out.push_str("# Host info (HostName / User / Port / IdentityFile) is read from\n");
    out.push_str("# ~/.ssh/config. This file only defines channels and per-host\n");
    out.push_str("# credentials that SSH config can't hold (passwords / passphrases).\n\n");

    if entries.is_empty() {
      out.push_str("# No usable Host blocks were found in ~/.ssh/config.\n");
      out.push_str("# Add at least one with HostName and User, then re-run `generate`.\n\n");
    } else {
      out.push_str("# --- Channel templates ---\n");
      out.push_str("# Uncomment the channels you want and fill in LOCAL:DEST ports.\n\n");
      for entry in entries {
        let target = entry.hostname.as_deref().unwrap_or("?");
        out.push_str(&format!("# Host alias: {} ({})\n", entry.host, target));
        out.push_str("# [[channels]]\n");
        out.push_str(&format!("# name = \"{}-tunnel\"\n", entry.host));
        out.push_str(&format!("# hostname = \"{}\"\n", entry.host));
        out.push_str("# ports = \"LOCAL:DEST\"   # e.g. \"8080:80\"\n\n");
      }

      let needs_auth: Vec<&ssh_config::SshConfigEntry> = entries
        .iter()
        .filter(|e| e.identity_file.is_none())
        .collect();
      if !needs_auth.is_empty() {
        out.push_str("# --- Credentials for password-auth hosts ---\n");
        out.push_str(
          "# These hosts have no IdentityFile in ~/.ssh/config; provide a password here.\n\n",
        );
        for entry in &needs_auth {
          out.push_str(&format!("# [auth.{}]\n", entry.host));
          out.push_str("# password = \"...\"\n\n");
        }
      } else {
        out.push_str("# --- Credential overrides (optional) ---\n");
        out.push_str("# Add a [auth.<alias>] table only when the alias needs a password\n");
        out.push_str("# or its IdentityFile is protected by a passphrase.\n");
        out.push_str("# [auth.example-alias]\n");
        out.push_str("# password = \"...\"\n");
        out.push_str("# # or: passphrase = \"...\"\n\n");
      }
    }

    out.push_str("# --- Reconnection settings ---\n");
    out.push_str("[reconnection]\n");
    out.push_str("max_retries = 0\n");
    out.push_str("initial_delay_secs = 1\n");
    out.push_str("max_delay_secs = 30\n");
    out.push_str("use_exponential_backoff = true\n");

    out
  }
}

fn resolve_auth(
  alias: &str,
  entry: &ssh_config::SshConfigEntry,
  override_: Option<&AuthOverride>,
) -> Result<AuthConfig> {
  let password = override_.and_then(|o| o.password.clone());
  let passphrase = override_.and_then(|o| o.passphrase.clone());

  if let Some(password) = password {
    return Ok(AuthConfig::Password { password });
  }

  if let Some(key_path) = entry.identity_file.clone() {
    return Ok(AuthConfig::Key {
      key_path,
      passphrase,
    });
  }

  Err(AppError::Config(format!(
    "Host '{}' has no `IdentityFile` in SSH config and no `[auth.{}].password` \
     in configs.toml — provide one or the other",
    alias, alias
  )))
}
