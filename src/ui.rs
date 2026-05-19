//! Terminal output helpers: colored badges, headers, tables.
//!
//! Colors auto-disable when stdout is not a TTY or `NO_COLOR` is set
//! (via `owo_colors::Stream::Stdout`).

use crate::config::{ChannelTypeParams, Direction};
use crate::service::{ChannelHealth, ChannelStatus, ServiceState, ServiceStatus};
use owo_colors::{OwoColorize, Stream::Stdout, Style};
use std::path::Path;

// ---------- Styles ----------

fn bold() -> Style {
  Style::new().bold()
}

fn dim() -> Style {
  Style::new().dimmed()
}

fn green_bold() -> Style {
  Style::new().green().bold()
}

fn red_bold() -> Style {
  Style::new().red().bold()
}

fn yellow_bold() -> Style {
  Style::new().yellow().bold()
}

fn cyan_bold() -> Style {
  Style::new().cyan().bold()
}

fn blue_bold() -> Style {
  Style::new().blue().bold()
}

fn magenta_bold() -> Style {
  Style::new().magenta().bold()
}

// ---------- Force toggle ----------

/// Force-disable colors (e.g. when `--no-color` is passed).
pub fn disable_colors() {
  owo_colors::set_override(false);
}

// ---------- Status lines ----------

/// Print a success line: `✓ <msg>` (green check).
pub fn success(msg: impl AsRef<str>) {
  println!(
    "{} {}",
    "✓".if_supports_color(Stdout, |t| t.style(green_bold())),
    msg.as_ref()
  );
}

/// Print a failure line: `✗ <msg>` (red cross).
pub fn fail(msg: impl AsRef<str>) {
  println!(
    "{} {}",
    "✗".if_supports_color(Stdout, |t| t.style(red_bold())),
    msg.as_ref()
  );
}

/// Print a warning line: `⚠ <msg>` (yellow).
pub fn warn(msg: impl AsRef<str>) {
  println!(
    "{} {}",
    "⚠".if_supports_color(Stdout, |t| t.style(yellow_bold())),
    msg.as_ref()
  );
}

/// Print an informational line: `ℹ <msg>` (cyan).
pub fn info(msg: impl AsRef<str>) {
  println!(
    "{} {}",
    "ℹ".if_supports_color(Stdout, |t| t.style(cyan_bold())),
    msg.as_ref()
  );
}

/// Print a hint line: `💡 <msg>` (dimmed body).
pub fn hint(msg: impl AsRef<str>) {
  println!(
    "💡 {}",
    msg.as_ref().if_supports_color(Stdout, |t| t.style(dim()))
  );
}

/// Print a progress step prefixed with `→`.
pub fn step(msg: impl AsRef<str>) {
  println!(
    "{} {}",
    "→".if_supports_color(Stdout, |t| t.style(blue_bold())),
    msg.as_ref()
  );
}

// ---------- Headers ----------

/// Bold underlined section header preceded by an emoji.
pub fn header(emoji: &str, title: &str) {
  let styled = title.if_supports_color(Stdout, |t| t.style(Style::new().bold().underline()));
  println!("\n{}  {}", emoji, styled);
}

/// Sub-header (bold, no underline, no leading blank line).
pub fn subheader(title: &str) {
  println!("{}", title.if_supports_color(Stdout, |t| t.style(bold())));
}

// ---------- Key / value ----------

/// `key: value` row with bold key, left-padded to 14 columns.
pub fn kv(key: &str, value: impl std::fmt::Display) {
  let label = format!("{:<14}", format!("{}:", key));
  println!(
    "  {} {}",
    label.if_supports_color(Stdout, |t| t.style(bold())),
    value
  );
}

/// Same as `kv` but `value` is rendered dim.
pub fn kv_dim(key: &str, value: impl std::fmt::Display) {
  let label = format!("{:<14}", format!("{}:", key));
  let val = value.to_string();
  println!(
    "  {} {}",
    label.if_supports_color(Stdout, |t| t.style(bold())),
    val.if_supports_color(Stdout, |t| t.style(dim()))
  );
}

// ---------- Service state ----------

/// Colored badge for a `ServiceState`.
pub fn state_badge(state: &ServiceState) -> String {
  let (icon, label, style) = match state {
    ServiceState::Running => ("●", "Running", green_bold()),
    ServiceState::Stopped => ("●", "Stopped", red_bold()),
    ServiceState::Starting => ("●", "Starting", yellow_bold()),
    ServiceState::Stopping => ("●", "Stopping", yellow_bold()),
    ServiceState::Error(_) => ("✗", "Error", red_bold()),
  };
  format!(
    "{} {}",
    icon.if_supports_color(Stdout, |t| t.style(style)),
    label.if_supports_color(Stdout, |t| t.style(style))
  )
}

/// `m / n` channel ratio rendered with a color cue based on coverage.
pub fn channels_ratio(active: usize, total: usize) -> String {
  let raw = format!("{}/{}", active, total);
  let style = if total == 0 {
    dim()
  } else if active == total {
    green_bold()
  } else if active == 0 {
    red_bold()
  } else {
    yellow_bold()
  };
  raw
    .if_supports_color(Stdout, |t| t.style(style))
    .to_string()
}

// ---------- Direction / endpoints ----------

/// Compact colored direction arrow: `→` for L→R, `←` for R→L.
pub fn direction_arrow(direction: Direction) -> String {
  let (sym, style) = match direction {
    Direction::LocalToRemote => ("→", green_bold()),
    Direction::RemoteToLocal => ("←", magenta_bold()),
  };
  sym
    .if_supports_color(Stdout, |t| t.style(style))
    .to_string()
}

/// Short readable direction label (`L→R` / `R→L`).
pub fn direction_short(direction: Direction) -> String {
  let (sym, style) = match direction {
    Direction::LocalToRemote => ("L→R", green_bold()),
    Direction::RemoteToLocal => ("R→L", magenta_bold()),
  };
  sym
    .if_supports_color(Stdout, |t| t.style(style))
    .to_string()
}

// ---------- Channel tables ----------

/// One row in the "Hosts found in SSH config" list rendered by `generate`.
pub fn host_entry_line(alias: &str, target: &str, key_info: &str, has_key: bool) {
  let alias_styled = alias.if_supports_color(Stdout, |t| t.style(cyan_bold()));
  let target_styled = target.if_supports_color(Stdout, |t| t.style(bold()));
  let arrow = "→".if_supports_color(Stdout, |t| t.style(blue_bold()));
  let key_style = if has_key { dim() } else { yellow_bold() };
  println!(
    "  {} [{}] {} {}  {}",
    "•".if_supports_color(Stdout, |t| t.style(blue_bold())),
    alias_styled,
    arrow,
    target_styled,
    key_info.if_supports_color(Stdout, |t| t.style(key_style)),
  );
}

/// Resolved channel entry (after merging config.toml + ~/.ssh/config).
pub fn resolved_channel_line(
  name: &str,
  username: &str,
  host: &str,
  port: u16,
  params: &ChannelTypeParams,
) {
  let endpoint = format!("{}@{}:{}", username, host, port);
  let detail = match params {
    ChannelTypeParams::DirectTcpIp {
      listen_host,
      local_port,
      dest_host,
      dest_port,
    } => format!(
      "{} {}:{} {} {}:{}",
      direction_short(Direction::LocalToRemote),
      listen_host,
      local_port,
      direction_arrow(Direction::LocalToRemote),
      dest_host,
      dest_port,
    ),
    ChannelTypeParams::ForwardedTcpIp {
      remote_bind_host,
      remote_bind_port,
      local_connect_host,
      local_connect_port,
    } => format!(
      "{} {}:{} {} {}:{}",
      direction_short(Direction::RemoteToLocal),
      remote_bind_host,
      remote_bind_port,
      direction_arrow(Direction::RemoteToLocal),
      local_connect_host,
      local_connect_port,
    ),
  };
  println!(
    "  {} {} {} {}",
    "•".if_supports_color(Stdout, |t| t.style(blue_bold())),
    name.if_supports_color(Stdout, |t| t.style(cyan_bold())),
    endpoint.if_supports_color(Stdout, |t| t.style(dim())),
    detail,
  );
}

// ---------- High-level blocks ----------

/// Colored badge for a `ChannelHealth`. Compact form for inline rendering;
/// the caller is responsible for any extra context (e.g. last_error tail).
pub fn health_badge(health: &ChannelHealth) -> String {
  let (icon, label, style) = match health {
    ChannelHealth::Stopped => ("●", "Stopped".to_string(), dim()),
    ChannelHealth::Connecting { attempt } => (
      "●",
      if *attempt > 1 {
        format!("Connecting #{}", attempt)
      } else {
        "Connecting".to_string()
      },
      yellow_bold(),
    ),
    ChannelHealth::Connected => ("●", "Connected".to_string(), green_bold()),
    ChannelHealth::Reconnecting { attempt, .. } => (
      "●",
      format!("Reconnecting #{}", attempt),
      yellow_bold(),
    ),
    ChannelHealth::Failed { .. } => ("✗", "Failed".to_string(), red_bold()),
  };
  format!(
    "{} {}",
    icon.if_supports_color(Stdout, |t| t.style(style)),
    label.if_supports_color(Stdout, |t| t.style(style))
  )
}

/// Render the per-channel health table inside Service Status. Each row shows
/// name, direction-tagged endpoints, and a colored health badge; rows in
/// Reconnecting/Failed get a second dimmed line with the last error.
pub fn print_channel_health_list(channels: &[ChannelStatus]) {
  if channels.is_empty() {
    return;
  }
  let count = format!("{}", channels.len());
  println!(
    "  {} {}{}{}",
    "Channels".if_supports_color(Stdout, |t| t.style(bold())),
    "(".if_supports_color(Stdout, |t| t.style(dim())),
    count.if_supports_color(Stdout, |t| t.style(cyan_bold())),
    "):".if_supports_color(Stdout, |t| t.style(dim())),
  );

  // Pad the channel name column so badges line up.
  let name_width = channels.iter().map(|c| c.name.len()).max().unwrap_or(0);

  for ch in channels {
    let badge = health_badge(&ch.health);
    let arrow = direction_arrow(ch.direction);
    // For local->remote: local listens, remote is dialed; for remote->local
    // the snapshot already swapped local/remote so this rendering is uniform.
    let endpoints = format!(
      "{} {} {} {}",
      direction_short(ch.direction),
      ch.local.if_supports_color(Stdout, |t| t.style(bold())),
      arrow,
      ch.remote.if_supports_color(Stdout, |t| t.style(bold())),
    );
    let name_padded = format!("{:width$}", ch.name, width = name_width);
    println!(
      "  {} {}  {}  {}",
      "•".if_supports_color(Stdout, |t| t.style(blue_bold())),
      name_padded.if_supports_color(Stdout, |t| t.style(cyan_bold())),
      endpoints,
      badge,
    );
    // Surface the failure cause on a second dim line — searching scrollback
    // for the matching `error` log lines is friction users shouldn't pay.
    let detail = match &ch.health {
      ChannelHealth::Reconnecting { last_error, .. } if !last_error.is_empty() => {
        Some(last_error.as_str())
      }
      ChannelHealth::Failed { error } if !error.is_empty() => Some(error.as_str()),
      _ => None,
    };
    if let Some(msg) = detail {
      // Two-space indent under the bullet; truncate so very long error
      // messages don't blow out the terminal.
      let trimmed: String = msg.chars().take(160).collect();
      let display = if msg.len() > 160 {
        format!("{}…", trimmed)
      } else {
        trimmed
      };
      println!(
        "      ↳ {}",
        display.if_supports_color(Stdout, |t| t.style(dim()))
      );
    }
  }
}

/// Print the full Service Status block (header + state line + per-channel
/// health table). Note rendering trails the table so it's the last thing
/// the user sees in watch mode.
pub fn print_service_status(
  status: &ServiceStatus,
  config_path: &Path,
  pid: Option<&str>,
  note: Option<&str>,
) {
  header("📋", "Service Status");
  kv("State", state_badge(&status.state));
  kv(
    "Channels",
    format!(
      "{} connected",
      channels_ratio(status.connected_count(), status.total_count())
    ),
  );
  kv_dim("Config", config_path.display());
  if let Some(pid) = pid {
    kv_dim("PID", pid);
  }
  if !status.channels.is_empty() {
    println!();
    print_channel_health_list(&status.channels);
  }
  if let Some(note) = note {
    println!();
    hint(note);
  }
}
