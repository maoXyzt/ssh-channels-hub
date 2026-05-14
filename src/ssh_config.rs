use crate::error::{AppError, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// SSH config entry parsed from ~/.ssh/config
#[derive(Debug, Clone)]
pub struct SshConfigEntry {
  /// Host alias/name
  pub host: String,
  /// Actual hostname
  pub hostname: Option<String>,
  /// SSH port
  pub port: Option<u16>,
  /// Username
  pub user: Option<String>,
  /// Identity file path
  pub identity_file: Option<PathBuf>,
}

/// Default values from Host "*" entry
#[derive(Debug, Clone, Default)]
struct SshConfigDefaults {
  port: Option<u16>,
  user: Option<String>,
  identity_file: Option<PathBuf>,
}

/// Parse SSH config file
pub fn parse_ssh_config(path: impl AsRef<Path>) -> Result<Vec<SshConfigEntry>> {
  let path = expand_tilde(path.as_ref())?;

  let content = std::fs::read_to_string(&path)
    .map_err(|e| AppError::Config(format!("Failed to read SSH config file: {}", e)))?;

  let entries = parse_ssh_config_content(&content)?;

  Ok(entries)
}

/// Get default SSH config path (~/.ssh/config)
pub fn default_ssh_config_path() -> PathBuf {
  if let Some(mut home) = dirs::home_dir() {
    home.push(".ssh");
    home.push("config");
    home
  } else {
    PathBuf::from("~/.ssh/config")
  }
}

/// Expand tilde (~) in path
fn expand_tilde(path: &Path) -> Result<PathBuf> {
  let path_str = path.to_string_lossy();

  if path_str == "~" {
    if let Some(home) = dirs::home_dir() {
      Ok(home)
    } else {
      Err(AppError::Config(
        "Cannot expand ~: home directory not found".to_string(),
      ))
    }
  } else if let Some(rest) = path_str.strip_prefix("~/") {
    if let Some(mut home) = dirs::home_dir() {
      home.push(rest);
      Ok(home)
    } else {
      Err(AppError::Config(
        "Cannot expand ~: home directory not found".to_string(),
      ))
    }
  } else {
    Ok(path.to_path_buf())
  }
}

/// Parse SSH config content.
///
/// Keywords are case-insensitive (`Host`, `host`, `HOST` all match), the
/// keyword-value separator may be whitespace or `=` (`Host=alias`), and
/// values can be quoted with `"`/`'` to preserve embedded spaces. A `Host`
/// line may list multiple aliases that share one config block — each
/// alias becomes its own entry. Within a block, the first occurrence of a
/// directive wins, matching `ssh_config(5)`.
fn parse_ssh_config_content(content: &str) -> Result<Vec<SshConfigEntry>> {
  let mut entries = Vec::new();
  let mut current_aliases: Vec<String> = Vec::new();
  let mut current_config: HashMap<String, String> = HashMap::new();
  let mut defaults = SshConfigDefaults::default();
  let mut is_default_host = false;

  let flush = |aliases: &mut Vec<String>,
               cfg: &mut HashMap<String, String>,
               is_default: &mut bool,
               defaults: &mut SshConfigDefaults,
               entries: &mut Vec<SshConfigEntry>| {
    if !aliases.is_empty() {
      if *is_default {
        // Multiple `Host *` blocks are merged, first occurrence wins (per ssh_config(5)).
        // `take().or(...)` so existing String / PathBuf values are moved, not cloned.
        let new = extract_defaults(cfg);
        defaults.port = defaults.port.or(new.port);
        defaults.user = defaults.user.take().or(new.user);
        defaults.identity_file = defaults.identity_file.take().or(new.identity_file);
      } else {
        for alias in aliases.iter() {
          if let Some(entry) = build_entry(alias, cfg, defaults) {
            entries.push(entry);
          }
        }
      }
    }
    aliases.clear();
    cfg.clear();
    *is_default = false;
  };

  for line in content.lines() {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }

    if let Some(host_part) = strip_keyword(line, "host") {
      flush(
        &mut current_aliases,
        &mut current_config,
        &mut is_default_host,
        &mut defaults,
        &mut entries,
      );

      let aliases = tokenize_value_list(host_part);
      if aliases.iter().any(|a| a == "*") {
        is_default_host = true;
        current_aliases = vec!["*".to_string()];
      } else {
        current_aliases = aliases;
      }
    } else if !current_aliases.is_empty()
      && let Some((key, value)) = parse_directive(line)
    {
      // First occurrence wins, per ssh_config(5).
      current_config.entry(key.to_lowercase()).or_insert(value);
    }
  }

  flush(
    &mut current_aliases,
    &mut current_config,
    &mut is_default_host,
    &mut defaults,
    &mut entries,
  );

  Ok(entries)
}

/// If `line` starts with `keyword` (case-insensitive) followed by whitespace or `=`,
/// return the rest of the line trimmed. Otherwise `None`.
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
  let kw_len = keyword.len();
  if line.len() < kw_len {
    return None;
  }
  let (head, rest) = line.split_at(kw_len);
  if !head.eq_ignore_ascii_case(keyword) {
    return None;
  }
  // The separator must be `=` (with any surrounding whitespace) or whitespace —
  // reject `HostName` when we're looking for `host`, but accept `host=foo`,
  // `host = foo`, and `host  foo`.
  if rest.is_empty() {
    None
  } else if let Some(after_eq) = rest.trim_start().strip_prefix('=') {
    Some(after_eq.trim_start())
  } else if rest.starts_with(char::is_whitespace) {
    Some(rest.trim_start())
  } else {
    None
  }
}

/// Split a `Host` value into individual aliases, honoring quoted segments.
fn tokenize_value_list(s: &str) -> Vec<String> {
  let mut out = Vec::new();
  let mut chars = s.chars().peekable();
  while let Some(&c) = chars.peek() {
    if c.is_whitespace() {
      chars.next();
      continue;
    }
    let token = read_token(&mut chars);
    if !token.is_empty() {
      out.push(token);
    }
  }
  out
}

/// Read one whitespace-delimited token, honoring `"..."` and `'...'` quotes.
fn read_token<I: Iterator<Item = char>>(chars: &mut std::iter::Peekable<I>) -> String {
  let mut out = String::new();
  while let Some(&c) = chars.peek() {
    if c.is_whitespace() {
      break;
    }
    if c == '"' || c == '\'' {
      let quote = c;
      chars.next();
      for qc in chars.by_ref() {
        if qc == quote {
          break;
        }
        out.push(qc);
      }
    } else {
      out.push(c);
      chars.next();
    }
  }
  out
}

/// Extract default values from Host "*" config
fn extract_defaults(config: &HashMap<String, String>) -> SshConfigDefaults {
  SshConfigDefaults {
    port: config.get("port").and_then(|p| p.parse::<u16>().ok()),
    user: config.get("user").cloned(),
    identity_file: config
      .get("identityfile")
      .and_then(|p| expand_tilde_in_path(p)),
  }
}

/// Parse a directive line such as `HostName example.com`, `Port=2222`, or
/// `IdentityFile "/path/with space"`. Returns the keyword and the de-quoted
/// value (multi-arg directives are joined with single spaces).
fn parse_directive(line: &str) -> Option<(&str, String)> {
  // Keyword runs until the first whitespace or `=`.
  let key_end = line
    .find(|c: char| c.is_whitespace() || c == '=')
    .filter(|&i| i > 0)?;
  let (key, rest) = line.split_at(key_end);
  // Allow `=` (with surrounding whitespace) or whitespace as the separator.
  let value_part = match rest.trim_start().strip_prefix('=') {
    Some(after_eq) => after_eq.trim_start(),
    None => rest.trim_start(),
  };
  if value_part.is_empty() {
    return None;
  }

  // De-quote every argument uniformly, then rejoin with single spaces.
  let tokens = tokenize_value_list(value_part);
  if tokens.is_empty() {
    return None;
  }
  Some((key, tokens.join(" ")))
}

/// Build SshConfigEntry from host name and config map, applying defaults
fn build_entry(
  host: &str,
  config: &HashMap<String, String>,
  defaults: &SshConfigDefaults,
) -> Option<SshConfigEntry> {
  // Skip entries without HostName (they might be patterns or incomplete)
  let hostname = config.get("hostname")?.clone();

  let port = config
    .get("port")
    .and_then(|p| p.parse::<u16>().ok())
    .or(defaults.port);

  let user = config
    .get("user")
    .cloned()
    .or_else(|| defaults.user.clone());

  let identity_file = config
    .get("identityfile")
    .and_then(|p| expand_tilde_in_path(p))
    .or_else(|| defaults.identity_file.clone());

  Some(SshConfigEntry {
    host: host.to_string(),
    hostname: Some(hostname),
    port,
    user,
    identity_file,
  })
}

/// Expand tilde in a path string
fn expand_tilde_in_path(path: &str) -> Option<PathBuf> {
  if let Some(rest) = path.strip_prefix("~/") {
    if let Some(mut home) = dirs::home_dir() {
      home.push(rest);
      Some(home)
    } else {
      None
    }
  } else {
    Some(PathBuf::from(path))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_simple_ssh_config() {
    let content = r#"
Host myserver
    HostName example.com
    Port 22
    User myuser
    IdentityFile ~/.ssh/id_rsa

Host myserver2
    HostName example2.com
    User user2
"#;

    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 2);

    assert_eq!(entries[0].host, "myserver");
    assert_eq!(entries[0].hostname, Some("example.com".to_string()));
    assert_eq!(entries[0].port, Some(22));
    assert_eq!(entries[0].user, Some("myuser".to_string()));

    assert_eq!(entries[1].host, "myserver2");
    assert_eq!(entries[1].hostname, Some("example2.com".to_string()));
    assert_eq!(entries[1].user, Some("user2".to_string()));
  }

  #[test]
  fn test_default_values_from_wildcard_host() {
    let content = r#"
Host *
    Port 2222
    User defaultuser
    IdentityFile ~/.ssh/default_key

Host myserver
    HostName example.com

Host myserver2
    HostName example2.com
    Port 22
    User customuser
"#;

    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 2);

    // myserver should inherit defaults from Host *
    assert_eq!(entries[0].host, "myserver");
    assert_eq!(entries[0].hostname, Some("example.com".to_string()));
    assert_eq!(entries[0].port, Some(2222)); // From Host *
    assert_eq!(entries[0].user, Some("defaultuser".to_string())); // From Host *
    assert!(entries[0].identity_file.is_some()); // Should have identity file from Host *

    // myserver2 should override defaults
    assert_eq!(entries[1].host, "myserver2");
    assert_eq!(entries[1].hostname, Some("example2.com".to_string()));
    assert_eq!(entries[1].port, Some(22)); // Overridden
    assert_eq!(entries[1].user, Some("customuser".to_string())); // Overridden
  }

  #[test]
  fn test_wildcard_host_not_included_in_entries() {
    let content = r#"
Host *
    Port 2222
    User defaultuser

Host myserver
    HostName example.com
"#;

    let entries = parse_ssh_config_content(content).unwrap();
    // Host "*" should not be included in entries
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].host, "myserver");
  }

  // --- ssh_config(5) compliance ---

  #[test]
  fn keywords_are_case_insensitive() {
    let content = r#"
host myserver
    HOSTNAME example.com
    user myuser
    identityFILE ~/.ssh/id_rsa
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].host, "myserver");
    assert_eq!(entries[0].hostname, Some("example.com".to_string()));
    assert_eq!(entries[0].user, Some("myuser".to_string()));
    assert!(entries[0].identity_file.is_some());
  }

  #[test]
  fn equals_sign_separator_is_accepted() {
    let content = r#"
Host=alias-eq
    HostName=example.com
    Port=2222
    User=myuser
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].host, "alias-eq");
    assert_eq!(entries[0].hostname, Some("example.com".to_string()));
    assert_eq!(entries[0].port, Some(2222));
    assert_eq!(entries[0].user, Some("myuser".to_string()));
  }

  #[test]
  fn host_line_with_multiple_aliases_creates_one_entry_per_alias() {
    let content = r#"
Host alpha beta gamma
    HostName shared.example.com
    User shareduser
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 3);
    let aliases: Vec<_> = entries.iter().map(|e| e.host.as_str()).collect();
    assert_eq!(aliases, vec!["alpha", "beta", "gamma"]);
    for e in &entries {
      assert_eq!(e.hostname, Some("shared.example.com".to_string()));
      assert_eq!(e.user, Some("shareduser".to_string()));
    }
  }

  #[test]
  fn quoted_value_strips_quotes_and_preserves_spaces() {
    let content = r#"
Host quoted
    HostName "example with space.com"
    IdentityFile "/tmp/key with spaces"
    User myuser
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
      entries[0].hostname,
      Some("example with space.com".to_string())
    );
    let key = entries[0].identity_file.as_ref().unwrap();
    let key_str = key.to_string_lossy();
    assert!(
      key_str.contains("key with spaces") && !key_str.contains('"'),
      "key path should be unquoted and preserve spaces, got: {key_str}"
    );
  }

  #[test]
  fn first_directive_wins_within_block() {
    let content = r#"
Host dup
    HostName first.example.com
    HostName second.example.com
    Port 2200
    Port 9999
    User firstuser
    User seconduser
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].hostname, Some("first.example.com".to_string()));
    assert_eq!(entries[0].port, Some(2200));
    assert_eq!(entries[0].user, Some("firstuser".to_string()));
  }

  #[test]
  fn multiple_wildcard_host_blocks_merge_with_first_wins() {
    // Two `Host *` blocks should compose, not clobber each other.
    let content = r#"
Host *
    Port 2222

Host *
    User defaultuser
    IdentityFile ~/.ssh/default_key

Host myserver
    HostName example.com
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.host, "myserver");
    assert_eq!(e.port, Some(2222), "Port from first Host *");
    assert_eq!(
      e.user,
      Some("defaultuser".to_string()),
      "User from second Host *"
    );
    assert!(e.identity_file.is_some(), "IdentityFile from second Host *");
  }

  #[test]
  fn first_wildcard_host_value_wins_when_repeated() {
    let content = r#"
Host *
    Port 2222

Host *
    Port 9999

Host myserver
    HostName example.com
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].port, Some(2222), "first Host * value should win");
  }

  #[test]
  fn equals_separator_tolerates_surrounding_whitespace() {
    let content = r#"
Host = padded-eq
    HostName  =  example.com
    Port    =    2222
    User=u
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.host, "padded-eq");
    assert_eq!(e.hostname, Some("example.com".to_string()));
    assert_eq!(e.port, Some(2222));
    assert_eq!(e.user, Some("u".to_string()));
  }

  #[test]
  fn directive_with_multiple_quoted_args_dequotes_all() {
    // Regression for inconsistent quoting in parse_directive: when a directive
    // has several quoted args, every one of them should be de-quoted (not just
    // the first), and the result should be space-joined.
    let content = r#"
Host multiarg
    HostName example.com
    User u
    SendEnv "VAR1" "VAR2"
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    // Round-trip through parse_directive directly (SendEnv isn't surfaced in
    // SshConfigEntry, but its parse path is shared with the ones that are).
    let (key, value) = parse_directive(r#"SendEnv "VAR1" "VAR2""#).unwrap();
    assert_eq!(key, "SendEnv");
    assert_eq!(value, "VAR1 VAR2", "all args should be de-quoted");

    let (key, value) = parse_directive(r#"IdentityFile "/path with space""#).unwrap();
    assert_eq!(key, "IdentityFile");
    assert_eq!(value, "/path with space");
  }

  #[test]
  fn hostname_keyword_is_not_confused_with_host() {
    // Regression guard: strip_keyword("HostName...", "host") must NOT match,
    // otherwise we'd start a new Host block on every HostName directive.
    let content = r#"
Host real-alias
    HostName real.example.com
    User u
"#;
    let entries = parse_ssh_config_content(content).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].host, "real-alias");
    assert_eq!(entries[0].hostname, Some("real.example.com".to_string()));
  }
}
