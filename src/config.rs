use crate::error::{AppError, Result};
use crate::ssh_config;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::path::PathBuf;

/// Tunnel direction. `LocalToRemote` ≈ `ssh -L`, `RemoteToLocal` ≈ `ssh -R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
  LocalToRemote,
  RemoteToLocal,
}

impl Direction {
  pub fn as_arrow(self) -> &'static str {
    match self {
      Direction::LocalToRemote => "local->remote",
      Direction::RemoteToLocal => "remote->local",
    }
  }
}

impl<'de> Deserialize<'de> for Direction {
  fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let s = String::deserialize(deserializer)?;
    match s.as_str() {
      "local->remote" => Ok(Direction::LocalToRemote),
      "remote->local" => Ok(Direction::RemoteToLocal),
      other => Err(serde::de::Error::custom(format!(
        "invalid direction '{}', expected \"local->remote\" or \"remote->local\"",
        other
      ))),
    }
  }
}

impl Serialize for Direction {
  fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    serializer.serialize_str(self.as_arrow())
  }
}

/// `host:port`, `[ipv6]:port`, or bare `port` (host defaults to 127.0.0.1).
///
/// Examples:
/// - `"3306"`             → `127.0.0.1:3306`
/// - `"0.0.0.0:8022"`     → `0.0.0.0:8022`
/// - `"db.internal:5432"` → `db.internal:5432`
/// - `"[::1]:3306"`       → `::1:3306`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
  pub host: String,
  pub port: u16,
}

impl Endpoint {
  fn parse(s: &str) -> Result<Self> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
      return Err(AppError::Config(
        "endpoint cannot be empty; expected \"port\" or \"host:port\"".to_string(),
      ));
    }

    if let Ok(port) = trimmed.parse::<u16>() {
      return Ok(Endpoint {
        host: "127.0.0.1".to_string(),
        port,
      });
    }

    if let Some(rest) = trimmed.strip_prefix('[') {
      let (host, port_str) = rest.split_once("]:").ok_or_else(|| {
        AppError::Config(format!(
          "invalid endpoint '{}': bracketed IPv6 must be in \"[addr]:port\" form",
          s
        ))
      })?;
      if host.is_empty() {
        return Err(AppError::Config(format!(
          "invalid endpoint '{}': empty IPv6 host",
          s
        )));
      }
      let port = port_str
        .parse::<u16>()
        .map_err(|e| AppError::Config(format!("invalid endpoint '{}': bad port: {}", s, e)))?;
      return Ok(Endpoint {
        host: host.to_string(),
        port,
      });
    }

    // rsplit so bare IPv6 (ambiguous) is forced into the [] branch above.
    let (host, port_str) = trimmed.rsplit_once(':').ok_or_else(|| {
      AppError::Config(format!(
        "invalid endpoint '{}': expected \"port\" or \"host:port\"",
        s
      ))
    })?;
    if host.is_empty() {
      return Err(AppError::Config(format!(
        "invalid endpoint '{}': host cannot be empty",
        s
      )));
    }
    let port = port_str
      .parse::<u16>()
      .map_err(|e| AppError::Config(format!("invalid endpoint '{}': bad port: {}", s, e)))?;
    Ok(Endpoint {
      host: host.to_string(),
      port,
    })
  }
}

impl<'de> Deserialize<'de> for Endpoint {
  fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let s = String::deserialize(deserializer)?;
    Endpoint::parse(&s).map_err(serde::de::Error::custom)
  }
}

impl Serialize for Endpoint {
  fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    serializer.serialize_str(&format!("{}:{}", self.host, self.port))
  }
}

/// Channel definition referencing an SSH config Host alias.
///
/// `local` / `remote` always name the address on that side; `direction` picks
/// which side listens:
/// - `local->remote` (ssh -L): listen on `local`, forward to `remote`.
/// - `remote->local` (ssh -R): server binds `remote`, bridge to `local` here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
  pub name: String,
  /// SSH config Host alias (the `<alias>` in a `Host <alias>` block).
  pub hostname: String,
  pub direction: Direction,
  pub local: Endpoint,
  pub remote: Endpoint,
}

/// SSH channel configuration (runtime — built by combining config.toml + ~/.ssh/config)
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
  pub auth: AuthConfig,
  pub params: ChannelTypeParams,
}

/// Parameters for each underlying SSH channel type. Names mirror RFC 4254.
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
    remote_bind_host: String,
    remote_bind_port: u16,
    local_connect_host: String,
    local_connect_port: u16,
  },
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

impl ConnectionConfig {
  /// `(host, port)` bound on this machine, or `None` when the bind happens on the server.
  pub fn local_listen_bind(&self) -> Option<(String, u16)> {
    match self.direction {
      Direction::LocalToRemote => Some((self.local.host.clone(), self.local.port)),
      Direction::RemoteToLocal => None,
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
  /// Order: current directory `config.toml`, then platform config dir `config.toml`.
  pub fn default_path_candidates() -> Vec<PathBuf> {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut candidates = vec![current_dir.join("config.toml")];
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
      .unwrap_or_else(|| PathBuf::from("config.toml"))
  }

  /// Resolved SSH config path (override from config.toml or platform default).
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

      let params = match conn.direction {
        Direction::LocalToRemote => ChannelTypeParams::DirectTcpIp {
          listen_host: conn.local.host.clone(),
          local_port: conn.local.port,
          dest_host: conn.remote.host.clone(),
          dest_port: conn.remote.port,
        },
        Direction::RemoteToLocal => ChannelTypeParams::ForwardedTcpIp {
          remote_bind_host: conn.remote.host.clone(),
          remote_bind_port: conn.remote.port,
          local_connect_host: conn.local.host.clone(),
          local_connect_port: conn.local.port,
        },
      };

      channels.push(ChannelConfig {
        name: conn.name.clone(),
        host,
        port,
        username,
        auth,
        params,
      });
    }

    Ok(channels)
  }

  /// Render a commented-out config.toml scaffold from SSH config entries.
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
      out.push_str("# direction: \"local->remote\" (ssh -L) or \"remote->local\" (ssh -R).\n");
      out.push_str("# local / remote: \"host:port\" (host defaults to 127.0.0.1 if omitted).\n\n");
      for entry in entries {
        let target = entry.hostname.as_deref().unwrap_or("?");
        out.push_str(&format!("# Host alias: {} ({})\n", entry.host, target));
        out.push_str("# [[channels]]\n");
        out.push_str(&format!("# name      = \"{}-tunnel\"\n", entry.host));
        out.push_str(&format!("# hostname  = \"{}\"\n", entry.host));
        out.push_str("# direction = \"local->remote\"\n");
        out.push_str("# local     = \"LOCAL_PORT\"        # e.g. \"8080\" or \"127.0.0.1:8080\"\n");
        out.push_str("# remote    = \"REMOTE_PORT\"       # e.g. \"80\" or \"127.0.0.1:80\"\n\n");
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
     in config.toml — provide one or the other",
    alias, alias
  )))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn endpoint_parses_bare_port() {
    let ep = Endpoint::parse("3306").unwrap();
    assert_eq!(ep.host, "127.0.0.1");
    assert_eq!(ep.port, 3306);
  }

  #[test]
  fn endpoint_parses_host_port() {
    let ep = Endpoint::parse("0.0.0.0:8022").unwrap();
    assert_eq!(ep.host, "0.0.0.0");
    assert_eq!(ep.port, 8022);
  }

  #[test]
  fn endpoint_parses_hostname() {
    let ep = Endpoint::parse("db.internal:5432").unwrap();
    assert_eq!(ep.host, "db.internal");
    assert_eq!(ep.port, 5432);
  }

  #[test]
  fn endpoint_parses_bracketed_ipv6() {
    let ep = Endpoint::parse("[::1]:3306").unwrap();
    assert_eq!(ep.host, "::1");
    assert_eq!(ep.port, 3306);
  }

  #[test]
  fn endpoint_rejects_empty() {
    assert!(Endpoint::parse("").is_err());
    assert!(Endpoint::parse("   ").is_err());
  }

  #[test]
  fn endpoint_rejects_missing_port() {
    assert!(Endpoint::parse("127.0.0.1:").is_err());
  }

  #[test]
  fn endpoint_rejects_missing_host() {
    assert!(Endpoint::parse(":3306").is_err());
  }

  #[test]
  fn endpoint_rejects_out_of_range_port() {
    assert!(Endpoint::parse("70000").is_err());
    assert!(Endpoint::parse("127.0.0.1:70000").is_err());
  }

  #[test]
  fn endpoint_rejects_garbage() {
    assert!(Endpoint::parse("not-a-port").is_err());
  }

  #[derive(Debug, Deserialize)]
  struct DirWrap {
    v: Direction,
  }

  #[derive(Debug, Deserialize, Serialize)]
  struct EpWrap {
    v: Endpoint,
  }

  #[test]
  fn direction_deserializes_both_arrows() {
    let l2r: DirWrap = toml::from_str(r#"v = "local->remote""#).unwrap();
    assert_eq!(l2r.v, Direction::LocalToRemote);

    let r2l: DirWrap = toml::from_str(r#"v = "remote->local""#).unwrap();
    assert_eq!(r2l.v, Direction::RemoteToLocal);
  }

  #[test]
  fn direction_rejects_invalid_value() {
    let err = toml::from_str::<DirWrap>(r#"v = "bogus""#).unwrap_err();
    let msg = err.to_string();
    assert!(
      msg.contains("local->remote") && msg.contains("remote->local"),
      "error should list valid options, got: {msg}"
    );
  }

  #[test]
  fn endpoint_round_trips_through_toml() {
    let parsed: EpWrap = toml::from_str(r#"v = "0.0.0.0:8022""#).unwrap();
    assert_eq!(parsed.v.host, "0.0.0.0");
    assert_eq!(parsed.v.port, 8022);

    let rendered = toml::to_string(&parsed).unwrap();
    assert!(rendered.contains("\"0.0.0.0:8022\""), "got: {rendered}");
  }
}
