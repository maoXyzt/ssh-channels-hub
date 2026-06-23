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
  /// Ordered ProxyJump chain. Empty when the target is dialed directly.
  /// Each hop is reached by opening a `direct-tcpip` channel on the previous
  /// hop's SSH session (first hop is dialed via plain TCP).
  pub proxy_jumps: Vec<JumpHopConfig>,
}

/// One hop in a ProxyJump chain. Built only from data the tool can read
/// non-interactively: a `Host <alias>` block in `~/.ssh/config` providing
/// HostName/User/Port and an unencrypted IdentityFile for publickey auth.
#[derive(Debug, Clone)]
pub struct JumpHopConfig {
  /// SSH config alias the hop was resolved from (for error messages).
  pub alias: String,
  pub host: String,
  pub port: u16,
  pub username: String,
  pub key_path: PathBuf,
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

    let default_keys = ssh_config::default_identity_file_candidates();

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

      // Honor ProxyCommand `ssh <alias> -W %h:%p` as a stand-in for ProxyJump
      // on SSH configs that predate OpenSSH 7.3. Only when ProxyJump is empty
      // — explicit ProxyJump always wins. Any other ProxyCommand shape errors.
      let entry_for_jump_resolution: ssh_config::SshConfigEntry;
      let entry_to_resolve: &ssh_config::SshConfigEntry = if !entry.proxy_jump.is_empty() {
        entry
      } else if let Some(cmd) = entry.proxy_command.as_deref() {
        let alias = parse_proxy_command_to_alias(cmd).ok_or_else(|| {
          AppError::Config(format!(
            "Channel host '{}' has `ProxyCommand {}`, which this tool does not understand. \
             Only one ProxyCommand shape is supported: `ssh <alias> -W %h:%p` (treated as \
             ProxyJump <alias>). Upgrade OpenSSH and switch to `ProxyJump <alias>`, or \
             rewrite the directive into that exact form.",
            conn.hostname, cmd
          ))
        })?;
        entry_for_jump_resolution = ssh_config::SshConfigEntry {
          proxy_jump: vec![alias],
          ..entry.clone()
        };
        &entry_for_jump_resolution
      } else {
        entry
      };

      let proxy_jumps =
        resolve_jump_chain(&conn.hostname, entry_to_resolve, &by_alias, &default_keys)?;

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
        proxy_jumps,
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

/// Recognize a single hard-coded ProxyCommand shape as a ProxyJump stand-in.
///
/// Supported: `ssh <alias> -W %h:%p` and `ssh -W %h:%p <alias>` (both ASCII
/// whitespace-separated; no extra flags). Returns `Some(alias)` on match,
/// `None` for any other shape. Deliberately narrow — the caller turns `None`
/// into a clear, actionable error.
pub(crate) fn parse_proxy_command_to_alias(cmd: &str) -> Option<String> {
  let tokens: Vec<&str> = cmd.split_whitespace().collect();
  if tokens.len() != 4 || tokens[0] != "ssh" {
    return None;
  }
  // The two accepted orderings differ only in where `-W %h:%p` sits.
  // Anywhere `-W` appears, the next token must be `%h:%p` and the remaining
  // token must be the alias (a non-flag bareword).
  let w_pos = tokens.iter().position(|t| *t == "-W")?;
  let percent_pos = w_pos + 1;
  if percent_pos >= tokens.len() || tokens[percent_pos] != "%h:%p" {
    return None;
  }
  let alias_pos = (1..tokens.len()).find(|&i| i != w_pos && i != percent_pos)?;
  let alias = tokens[alias_pos];
  if alias.is_empty() || alias.starts_with('-') || alias.contains('@') || alias.contains(':') {
    return None;
  }
  Some(alias.to_string())
}

/// Resolve the ordered ProxyJump chain for a channel.
///
/// Scope: each token in the entry's `ProxyJump` directive must be a Host alias
/// that has its own `Host <alias>` block in `~/.ssh/config`. Raw `user@host:port`
/// forms are rejected with a clear message asking the user to define an alias.
/// Each hop must have HostName, User, and an IdentityFile (explicit on the
/// alias, inherited from `Host *`, or one of the default `~/.ssh/id_*` files).
/// Passphrase-protected keys and password auth are not supported on jumps —
/// the encryption check happens at connect time when the key is actually loaded.
fn resolve_jump_chain(
  channel_alias: &str,
  entry: &ssh_config::SshConfigEntry,
  by_alias: &HashMap<&str, &ssh_config::SshConfigEntry>,
  default_keys: &[PathBuf],
) -> Result<Vec<JumpHopConfig>> {
  let mut chain = Vec::with_capacity(entry.proxy_jump.len());

  for token in &entry.proxy_jump {
    if token.contains('@') || token.contains(':') {
      return Err(AppError::Config(format!(
        "Channel host '{}' has ProxyJump '{}' written as a raw target. \
         This tool only supports ProxyJump values that reference a `Host <alias>` \
         block in {}. Define one for '{}' and replace the ProxyJump value with \
         the alias.",
        channel_alias, token, "~/.ssh/config", token,
      )));
    }

    let jump_entry = by_alias.get(token.as_str()).copied().ok_or_else(|| {
      AppError::Config(format!(
        "Channel host '{}' has ProxyJump '{}', but no `Host {}` block exists in \
         ~/.ssh/config. Define it (HostName + User + IdentityFile) or remove \
         the ProxyJump reference.",
        channel_alias, token, token,
      ))
    })?;

    let host = jump_entry.hostname.clone().ok_or_else(|| {
      AppError::Config(format!(
        "ProxyJump alias '{}' (used by channel host '{}') is missing `HostName`",
        token, channel_alias
      ))
    })?;
    let username = jump_entry.user.clone().ok_or_else(|| {
      AppError::Config(format!(
        "ProxyJump alias '{}' (used by channel host '{}') is missing `User`",
        token, channel_alias
      ))
    })?;
    let port = jump_entry.port.unwrap_or(22);

    let key_path = jump_entry
      .identity_file
      .clone()
      .or_else(|| default_keys.first().cloned())
      .ok_or_else(|| {
        AppError::Config(format!(
          "ProxyJump alias '{}' (used by channel host '{}') has no `IdentityFile` \
           and no default key (`~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`, `id_dsa`) \
           exists. This tool supports publickey-only auth on jump hosts.",
          token, channel_alias
        ))
      })?;

    chain.push(JumpHopConfig {
      alias: token.clone(),
      host,
      port,
      username,
      key_path,
    });
  }

  Ok(chain)
}

/// Outcome of `check_jump_preflight`: things validate should surface before the
/// daemon starts and discovers them the hard way at connect time.
#[derive(Debug, Default, Clone)]
pub struct JumpPreflightReport {
  /// Hard failures — the daemon will not be able to connect. Validate should
  /// fail the run.
  pub errors: Vec<String>,
  /// Soft issues — the user can fix them out-of-band (typically by running
  /// `ssh-keyscan` or `ssh <alias>` once). Validate should print them and
  /// still succeed.
  pub warnings: Vec<String>,
}

/// Environmental checks for ProxyJump chains that are cheap enough to run at
/// validate time:
///
/// - **Error** if a jump hop's `IdentityFile` doesn't exist on disk — the key
///   was resolved (explicit / `Host *` / default), but the file isn't there.
/// - **Warning** if a jump hop's `(host, port)` has no entry in `known_hosts` —
///   the daemon's strict known_hosts check will refuse the handshake. We warn
///   instead of erroring because it's an environment issue the user can fix
///   without touching config, and we don't want validate to fail in CI just
///   because the runner's known_hosts isn't seeded.
///
/// `known_hosts_override` lets tests point at a fixture file; pass `None` in
/// production to use the per-user default (`~/.ssh/known_hosts`).
pub fn check_jump_preflight(
  channels: &[ChannelConfig],
  known_hosts_override: Option<&std::path::Path>,
) -> JumpPreflightReport {
  use std::collections::HashSet;

  let mut report = JumpPreflightReport::default();

  // Dedupe by the natural key: same identity file (or same host:port) only
  // gets one diagnostic regardless of how many channels share the bastion.
  let mut seen_keys: HashSet<PathBuf> = HashSet::new();
  let mut seen_hosts: HashSet<(String, u16)> = HashSet::new();

  let known_hosts_present = match known_hosts_override {
    Some(p) => p.exists(),
    None => dirs::home_dir()
      .map(|h| h.join(".ssh").join("known_hosts"))
      .map(|p| p.exists())
      .unwrap_or(false),
  };

  let any_jump = channels.iter().any(|c| !c.proxy_jumps.is_empty());
  if any_jump && !known_hosts_present {
    let where_ = known_hosts_override
      .map(|p| p.display().to_string())
      .unwrap_or_else(|| "~/.ssh/known_hosts".to_string());
    report.warnings.push(format!(
      "ProxyJump is in use but {} does not exist. \
       Jumps will be refused at connect time (strict known_hosts). \
       Run `ssh-keyscan -p <port> <host> >> ~/.ssh/known_hosts` for each jump host first.",
      where_
    ));
  }

  for ch in channels {
    for hop in &ch.proxy_jumps {
      let key_path = hop.key_path.clone();
      if seen_keys.insert(key_path.clone()) && !key_path.exists() {
        report.errors.push(format!(
          "Channel '{}' → ProxyJump '{}': IdentityFile '{}' does not exist on disk",
          ch.name,
          hop.alias,
          key_path.display()
        ));
      }

      let host_key = (hop.host.clone(), hop.port);
      if known_hosts_present && seen_hosts.insert(host_key) {
        let lookup = match known_hosts_override {
          Some(p) => russh_keys::known_host_keys_path(&hop.host, hop.port, p),
          None => russh_keys::known_host_keys(&hop.host, hop.port),
        };
        match lookup {
          Ok(v) if v.is_empty() => {
            report.warnings.push(format!(
              "Channel '{}' → ProxyJump '{}': no entry for {}:{} in known_hosts. \
               Fix: `ssh-keyscan -p {} {} >> ~/.ssh/known_hosts` or `ssh {}` once.",
              ch.name, hop.alias, hop.host, hop.port, hop.port, hop.host, hop.alias
            ));
          }
          Ok(_) => {}
          Err(e) => {
            report.warnings.push(format!(
              "Channel '{}' → ProxyJump '{}': failed to read known_hosts for {}:{}: {}",
              ch.name, hop.alias, hop.host, hop.port, e
            ));
          }
        }
      }
    }
  }

  report
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

  // --- resolve_jump_chain ---

  fn make_entry(
    host: &str,
    hostname: Option<&str>,
    user: Option<&str>,
    port: Option<u16>,
    identity_file: Option<PathBuf>,
    proxy_jump: Vec<String>,
  ) -> ssh_config::SshConfigEntry {
    ssh_config::SshConfigEntry {
      host: host.to_string(),
      hostname: hostname.map(String::from),
      user: user.map(String::from),
      port,
      identity_file,
      proxy_jump,
      proxy_command: None,
    }
  }

  fn make_entry_with_proxy_command(
    host: &str,
    hostname: Option<&str>,
    user: Option<&str>,
    proxy_command: Option<&str>,
  ) -> ssh_config::SshConfigEntry {
    ssh_config::SshConfigEntry {
      host: host.to_string(),
      hostname: hostname.map(String::from),
      user: user.map(String::from),
      port: None,
      identity_file: None,
      proxy_jump: vec![],
      proxy_command: proxy_command.map(String::from),
    }
  }

  #[test]
  fn jump_chain_empty_when_no_proxy_jump() {
    let target = make_entry("t", Some("t.example.com"), Some("u"), None, None, vec![]);
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      std::iter::once(("t", &target)).collect();
    let chain = resolve_jump_chain("t", &target, &by_alias, &[]).unwrap();
    assert!(chain.is_empty());
  }

  #[test]
  fn jump_chain_single_alias_uses_alias_identity_file() {
    let bastion_key = PathBuf::from("/keys/bastion");
    let bastion = make_entry(
      "bastion",
      Some("bastion.example.com"),
      Some("bu"),
      Some(2200),
      Some(bastion_key.clone()),
      vec![],
    );
    let target = make_entry(
      "t",
      Some("t.example.com"),
      Some("u"),
      None,
      None,
      vec!["bastion".to_string()],
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      [("t", &target), ("bastion", &bastion)]
        .into_iter()
        .collect();
    let chain = resolve_jump_chain("t", &target, &by_alias, &[]).unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].alias, "bastion");
    assert_eq!(chain[0].host, "bastion.example.com");
    assert_eq!(chain[0].port, 2200);
    assert_eq!(chain[0].username, "bu");
    assert_eq!(chain[0].key_path, bastion_key);
  }

  #[test]
  fn jump_chain_multi_hop_preserves_order() {
    let k = PathBuf::from("/keys/k");
    let alpha = make_entry(
      "alpha",
      Some("a.example.com"),
      Some("ua"),
      None,
      Some(k.clone()),
      vec![],
    );
    let beta = make_entry(
      "beta",
      Some("b.example.com"),
      Some("ub"),
      None,
      Some(k.clone()),
      vec![],
    );
    let target = make_entry(
      "t",
      Some("t.example.com"),
      Some("u"),
      None,
      None,
      vec!["alpha".to_string(), "beta".to_string()],
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      [("t", &target), ("alpha", &alpha), ("beta", &beta)]
        .into_iter()
        .collect();
    let chain = resolve_jump_chain("t", &target, &by_alias, &[]).unwrap();
    let aliases: Vec<_> = chain.iter().map(|h| h.alias.as_str()).collect();
    assert_eq!(aliases, vec!["alpha", "beta"]);
  }

  #[test]
  fn jump_chain_rejects_raw_user_at_host_port_form() {
    let target = make_entry(
      "t",
      Some("t.example.com"),
      Some("u"),
      None,
      None,
      vec!["admin@jump.example.com:2222".to_string()],
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      std::iter::once(("t", &target)).collect();
    let err = resolve_jump_chain("t", &target, &by_alias, &[]).unwrap_err();
    let msg = err.to_string();
    assert!(
      msg.contains("raw target") && msg.contains("Host <alias>"),
      "expected raw-form rejection message, got: {msg}"
    );
  }

  #[test]
  fn jump_chain_rejects_unknown_alias() {
    let target = make_entry(
      "t",
      Some("t.example.com"),
      Some("u"),
      None,
      None,
      vec!["missing".to_string()],
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      std::iter::once(("t", &target)).collect();
    let err = resolve_jump_chain("t", &target, &by_alias, &[]).unwrap_err();
    let msg = err.to_string();
    assert!(
      msg.contains("missing") && msg.contains("no `Host"),
      "expected unknown-alias message, got: {msg}"
    );
  }

  #[test]
  fn jump_chain_falls_back_to_default_key_when_no_identity_file() {
    let default = PathBuf::from("/home/u/.ssh/id_ed25519");
    let bastion = make_entry(
      "bastion",
      Some("b.example.com"),
      Some("u"),
      None,
      None, // no IdentityFile
      vec![],
    );
    let target = make_entry(
      "t",
      Some("t.example.com"),
      Some("u"),
      None,
      None,
      vec!["bastion".to_string()],
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      [("t", &target), ("bastion", &bastion)]
        .into_iter()
        .collect();
    let chain =
      resolve_jump_chain("t", &target, &by_alias, std::slice::from_ref(&default)).unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].key_path, default);
  }

  #[test]
  fn jump_chain_errors_when_no_key_anywhere() {
    let bastion = make_entry(
      "bastion",
      Some("b.example.com"),
      Some("u"),
      None,
      None, // no IdentityFile
      vec![],
    );
    let target = make_entry(
      "t",
      Some("t.example.com"),
      Some("u"),
      None,
      None,
      vec!["bastion".to_string()],
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      [("t", &target), ("bastion", &bastion)]
        .into_iter()
        .collect();
    let err = resolve_jump_chain("t", &target, &by_alias, &[]).unwrap_err();
    let msg = err.to_string();
    assert!(
      msg.contains("IdentityFile") && msg.contains("publickey-only"),
      "expected missing-key message, got: {msg}"
    );
  }

  #[test]
  fn jump_chain_errors_when_alias_missing_user() {
    let bastion = make_entry(
      "bastion",
      Some("b.example.com"),
      None, // missing User
      None,
      Some(PathBuf::from("/k")),
      vec![],
    );
    let target = make_entry(
      "t",
      Some("t.example.com"),
      Some("u"),
      None,
      None,
      vec!["bastion".to_string()],
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      [("t", &target), ("bastion", &bastion)]
        .into_iter()
        .collect();
    let err = resolve_jump_chain("t", &target, &by_alias, &[]).unwrap_err();
    let msg = err.to_string();
    assert!(
      msg.contains("missing `User`"),
      "expected missing-user message, got: {msg}"
    );
  }

  // --- parse_proxy_command_to_alias ---

  #[test]
  fn proxy_command_accepts_alias_before_w_flag() {
    assert_eq!(
      parse_proxy_command_to_alias("ssh bastion -W %h:%p"),
      Some("bastion".to_string())
    );
  }

  #[test]
  fn proxy_command_accepts_alias_after_w_flag() {
    assert_eq!(
      parse_proxy_command_to_alias("ssh -W %h:%p bastion"),
      Some("bastion".to_string())
    );
  }

  #[test]
  fn proxy_command_tolerates_extra_whitespace() {
    assert_eq!(
      parse_proxy_command_to_alias("  ssh   bastion   -W   %h:%p  "),
      Some("bastion".to_string())
    );
  }

  #[test]
  fn proxy_command_rejects_extra_flags() {
    assert!(parse_proxy_command_to_alias("ssh -q bastion -W %h:%p").is_none());
    assert!(parse_proxy_command_to_alias("ssh bastion -W %h:%p -q").is_none());
  }

  #[test]
  fn proxy_command_rejects_non_ssh_executable() {
    assert!(parse_proxy_command_to_alias("nc bastion %p").is_none());
    assert!(parse_proxy_command_to_alias("/usr/bin/ssh bastion -W %h:%p").is_none());
  }

  #[test]
  fn proxy_command_rejects_raw_user_host_form() {
    // The alias must be a bareword: `admin@host` or `host:22` shouldn't match
    // because downstream resolver only accepts Host aliases.
    assert!(parse_proxy_command_to_alias("ssh admin@bastion -W %h:%p").is_none());
    assert!(parse_proxy_command_to_alias("ssh bastion:2222 -W %h:%p").is_none());
  }

  #[test]
  fn proxy_command_rejects_wrong_w_target() {
    assert!(parse_proxy_command_to_alias("ssh bastion -W %h").is_none());
    assert!(parse_proxy_command_to_alias("ssh bastion -W some:port").is_none());
  }

  #[test]
  fn proxy_command_rejects_alternate_tools() {
    assert!(parse_proxy_command_to_alias("nc -X connect bastion 22").is_none());
    assert!(parse_proxy_command_to_alias("ssh-keygen bastion").is_none());
  }

  // build_channels-side smoke: a ProxyCommand alias is treated as ProxyJump
  // and resolves through resolve_jump_chain like any normal alias would.
  #[test]
  fn proxy_command_threads_through_to_resolve_jump_chain_via_clone() {
    use crate::ssh_config;
    let bastion_key = PathBuf::from("/keys/bastion");
    let bastion = make_entry(
      "bastion",
      Some("bastion.example.com"),
      Some("bu"),
      None,
      Some(bastion_key.clone()),
      vec![],
    );
    let target = make_entry_with_proxy_command(
      "t",
      Some("t.example.com"),
      Some("u"),
      Some("ssh bastion -W %h:%p"),
    );
    let by_alias: HashMap<&str, &ssh_config::SshConfigEntry> =
      [("t", &target), ("bastion", &bastion)]
        .into_iter()
        .collect();

    // Simulate the build_channels rewrite: turn proxy_command into proxy_jump
    // and feed the clone to resolve_jump_chain.
    let alias = parse_proxy_command_to_alias(target.proxy_command.as_ref().unwrap()).unwrap();
    let target_eff = ssh_config::SshConfigEntry {
      proxy_jump: vec![alias],
      ..target.clone()
    };
    let chain = resolve_jump_chain("t", &target_eff, &by_alias, &[]).unwrap();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].alias, "bastion");
    assert_eq!(chain[0].host, "bastion.example.com");
  }

  // --- check_jump_preflight ---

  fn make_channel(name: &str, proxy_jumps: Vec<JumpHopConfig>) -> ChannelConfig {
    ChannelConfig {
      name: name.to_string(),
      host: "target.example.com".to_string(),
      port: 22,
      username: "u".to_string(),
      auth: AuthConfig::Password {
        password: "x".to_string(),
      },
      params: ChannelTypeParams::DirectTcpIp {
        listen_host: "127.0.0.1".to_string(),
        local_port: 3306,
        dest_host: "127.0.0.1".to_string(),
        dest_port: 3306,
      },
      proxy_jumps,
    }
  }

  #[test]
  fn preflight_empty_when_no_jumps() {
    let ch = make_channel("c", vec![]);
    let report = check_jump_preflight(&[ch], None);
    assert!(report.errors.is_empty());
    assert!(report.warnings.is_empty());
  }

  #[test]
  fn preflight_errors_on_missing_identity_file() {
    let missing = std::env::temp_dir().join(format!(
      "ssh-channels-hub-test-missing-{}",
      std::process::id()
    ));
    // make doubly sure the path doesn't exist (don't litter, don't pre-create)
    let _ = std::fs::remove_file(&missing);
    let hop = JumpHopConfig {
      alias: "bastion".to_string(),
      host: "b.example.com".to_string(),
      port: 22,
      username: "u".to_string(),
      key_path: missing.clone(),
    };
    let ch = make_channel("c", vec![hop]);

    // Use a fake known_hosts that exists to isolate the IdentityFile path.
    let dir = std::env::temp_dir().join(format!("ssh-channels-hub-test-kh-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let kh = dir.join("known_hosts");
    std::fs::write(&kh, "").unwrap();

    let report = check_jump_preflight(&[ch], Some(&kh));
    assert_eq!(report.errors.len(), 1);
    assert!(
      report.errors[0].contains("IdentityFile") && report.errors[0].contains("does not exist"),
      "got: {}",
      report.errors[0]
    );

    let _ = std::fs::remove_file(&kh);
    let _ = std::fs::remove_dir(&dir);
  }

  #[test]
  fn preflight_warns_when_known_hosts_missing_overall() {
    // Even existing IdentityFile — if known_hosts itself doesn't exist, we
    // emit one top-level warning instead of N per-host ones.
    let existing_key =
      std::env::temp_dir().join(format!("ssh-channels-hub-test-key-{}", std::process::id()));
    std::fs::write(&existing_key, "fake").unwrap();
    let hop = JumpHopConfig {
      alias: "bastion".to_string(),
      host: "b.example.com".to_string(),
      port: 22,
      username: "u".to_string(),
      key_path: existing_key.clone(),
    };
    let ch = make_channel("c", vec![hop]);

    let nonexistent_kh = std::env::temp_dir().join(format!(
      "ssh-channels-hub-test-kh-nope-{}",
      std::process::id()
    ));
    let _ = std::fs::remove_file(&nonexistent_kh);

    let report = check_jump_preflight(&[ch], Some(&nonexistent_kh));
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert!(
      report
        .warnings
        .iter()
        .any(|w| w.contains("does not exist") && w.contains("strict known_hosts")),
      "warnings: {:?}",
      report.warnings
    );

    let _ = std::fs::remove_file(&existing_key);
  }

  #[test]
  fn preflight_warns_when_jump_host_missing_from_known_hosts() {
    let existing_key =
      std::env::temp_dir().join(format!("ssh-channels-hub-test-key2-{}", std::process::id()));
    std::fs::write(&existing_key, "fake").unwrap();
    let hop = JumpHopConfig {
      alias: "bastion".to_string(),
      host: "b.example.com".to_string(),
      port: 22,
      username: "u".to_string(),
      key_path: existing_key.clone(),
    };
    let ch = make_channel("c", vec![hop]);

    let kh_path = std::env::temp_dir().join(format!(
      "ssh-channels-hub-test-kh-empty-{}",
      std::process::id()
    ));
    // Empty but existing known_hosts — file present, host absent.
    std::fs::write(&kh_path, "").unwrap();

    let report = check_jump_preflight(&[ch], Some(&kh_path));
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert!(
      report
        .warnings
        .iter()
        .any(|w| w.contains("no entry for b.example.com:22")),
      "warnings: {:?}",
      report.warnings
    );

    let _ = std::fs::remove_file(&existing_key);
    let _ = std::fs::remove_file(&kh_path);
  }

  #[test]
  fn preflight_dedupes_shared_bastion_across_channels() {
    let missing = std::env::temp_dir().join(format!(
      "ssh-channels-hub-test-shared-{}",
      std::process::id()
    ));
    let _ = std::fs::remove_file(&missing);
    let hop = JumpHopConfig {
      alias: "bastion".to_string(),
      host: "b.example.com".to_string(),
      port: 22,
      username: "u".to_string(),
      key_path: missing.clone(),
    };
    let a = make_channel("a", vec![hop.clone()]);
    let b = make_channel("b", vec![hop.clone()]);
    let c = make_channel("c", vec![hop]);

    let kh = std::env::temp_dir().join(format!(
      "ssh-channels-hub-test-dedupe-kh-{}",
      std::process::id()
    ));
    std::fs::write(&kh, "").unwrap();

    let report = check_jump_preflight(&[a, b, c], Some(&kh));
    // Shared key path → one error, not three.
    assert_eq!(
      report.errors.len(),
      1,
      "expected single dedup'd error, got: {:?}",
      report.errors
    );
    // Shared (host, port) → one known_hosts warning, not three.
    let kh_warnings: Vec<_> = report
      .warnings
      .iter()
      .filter(|w| w.contains("no entry for"))
      .collect();
    assert_eq!(
      kh_warnings.len(),
      1,
      "expected single dedup'd kh warning, got: {:?}",
      kh_warnings
    );

    let _ = std::fs::remove_file(&kh);
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
