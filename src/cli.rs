use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// SSH Channels Hub — manage SSH port-forwarding tunnels with auto-reconnect.
///
/// Workflow:
///   1. `generate` — scaffold a config.toml from ~/.ssh/config
///   2. edit config.toml — uncomment channels, fill in ports
///   3. `validate` — sanity-check the config against ~/.ssh/config
///   4. `start -D` — run as a background daemon
///   5. `status` / `test` — inspect the live state
///
/// Configuration lookup (when `-c` is not given):
///   ./config.toml        →  $XDG_CONFIG_HOME/ssh-channels-hub/config.toml
#[derive(Parser)]
#[command(name = "ssh-channels-hub")]
#[command(version)]
#[command(
  about = "Manage SSH port-forwarding tunnels with auto-reconnect",
  long_about = None,
)]
#[command(propagate_version = true)]
#[command(arg_required_else_help = true)]
pub struct Cli {
  #[command(subcommand)]
  pub command: Commands,

  /// Path to config.toml (default: ./config.toml or platform config dir).
  #[arg(short, long, global = true, value_name = "FILE")]
  pub config: Option<PathBuf>,

  /// Enable debug-level logging (overrides RUST_LOG=info).
  #[arg(short, long, global = true)]
  pub debug: bool,

  /// Disable ANSI colors in output (also honors NO_COLOR env var).
  #[arg(long, global = true)]
  pub no_color: bool,
}

#[derive(Subcommand)]
pub enum Commands {
  /// Start the service (foreground, or detached with -D).
  #[command(long_about = "\
Bring up every channel defined in config.toml.

By default runs in the foreground until Ctrl+C; pass -D to detach.
While running, `status` / `stop` / `restart` connect over a local
IPC socket recorded next to config.toml.")]
  Start {
    /// Detach as a background daemon (writes PID/port file next to config).
    #[arg(short = 'D', long)]
    daemon: bool,
  },

  /// Stop the running service (signals the daemon via IPC).
  Stop,

  /// Stop the running service, then start it again in daemon mode.
  Restart,

  /// Show whether the service is running, plus configured channels.
  #[command(long_about = "\
Connects to the running daemon over IPC to report live state, with
per-channel health (Connected / Reconnecting / Failed / Stopped).

When no daemon is running, falls back to printing the channels
declared in config.toml so you can still see the intended setup.

With --watch, re-renders periodically until interrupted (Ctrl+C);
useful for monitoring a fragile link.")]
  Status {
    /// Re-render every <interval> seconds until Ctrl+C.
    #[arg(short = 'w', long)]
    watch: bool,
    /// Watch refresh interval in seconds (minimum 1, only used with --watch).
    #[arg(short = 'n', long, default_value_t = 2, value_name = "SECONDS")]
    interval: u64,
  },

  /// Validate config.toml — resolves each channel against ~/.ssh/config.
  #[command(long_about = "\
Parses config.toml and resolves every `[[channels]]` entry against
~/.ssh/config. Catches missing host aliases, missing HostName/User,
unparseable ports, and hosts that need a password but have no
[auth.<alias>] block.")]
  Validate {
    /// config.toml to validate (overrides -c).
    config: Option<PathBuf>,
  },

  /// Scaffold a config.toml from your SSH config (~/.ssh/config).
  #[command(long_about = "\
Reads ~/.ssh/config and writes a config.toml with one commented-out
`[[channels]]` template per Host alias, plus stub [auth.<alias>]
blocks for hosts that have no IdentityFile.")]
  Generate {
    /// SSH config to read (default: ~/.ssh/config).
    #[arg(short, long, value_name = "FILE")]
    ssh_config: Option<PathBuf>,

    /// Output path for the scaffolded TOML (default: ./config.toml).
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
  },

  /// List SSH config Host aliases and whether this tool can use them.
  #[command(long_about = "\
Scans SSH config and reports each `Host <alias>` block as supported or
unsupported for ssh-channels-hub. Use this before writing channels to find
missing HostName/User fields, unsupported ProxyJump forms, and hosts that
will need a password in config.toml.")]
  Hosts {
    /// SSH config to read (default: ~/.ssh/config).
    #[arg(short, long, value_name = "FILE")]
    ssh_config: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = HostOutputFormat::Table)]
    format: HostOutputFormat,
  },

  /// Probe each local→remote channel by connecting to its local port.
  #[command(long_about = "\
For every `local->remote` channel, try a TCP connect to the local
listen address and verify the tunnel is alive. `remote->local`
channels are skipped — those can only be verified from the SSH
server side.")]
  Test {
    /// config.toml to test against (overrides -c).
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
  },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HostOutputFormat {
  Table,
  Json,
}
