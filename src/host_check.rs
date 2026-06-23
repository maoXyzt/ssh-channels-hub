use crate::config::parse_proxy_command_to_alias;
use crate::ssh_config::{self, SshConfigEntry};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostSupportStatus {
  Supported,
  Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostSupportReport {
  pub alias: String,
  pub hostname: Option<String>,
  pub status: HostSupportStatus,
  pub reasons: Vec<String>,
  pub warnings: Vec<String>,
}

/// Analyze every parsed SSH config Host alias against the capabilities this
/// tool can use non-interactively.
pub fn analyze_hosts(entries: &[SshConfigEntry]) -> Vec<HostSupportReport> {
  let by_alias: HashMap<&str, &SshConfigEntry> = entries
    .iter()
    .map(|entry| (entry.host.as_str(), entry))
    .collect();
  let default_keys = ssh_config::default_identity_file_candidates();
  let default_key_count = default_keys.len();

  entries
    .iter()
    .map(|entry| analyze_host(entry, &by_alias, default_key_count))
    .collect()
}

fn analyze_host(
  entry: &SshConfigEntry,
  by_alias: &HashMap<&str, &SshConfigEntry>,
  default_key_count: usize,
) -> HostSupportReport {
  let mut failure_reasons = Vec::new();
  let mut success_reasons = Vec::new();
  let mut warnings = Vec::new();

  if entry.hostname.is_none() {
    failure_reasons.push("missing `HostName`".to_string());
  }

  if entry.user.is_none() {
    failure_reasons.push("missing `User`".to_string());
  }

  if entry.hostname.is_some() && entry.user.is_some() {
    success_reasons.push("HostName and User are present".to_string());
  }

  if entry.identity_file.is_some() {
    success_reasons.push("authentication can use `IdentityFile`".to_string());
  } else if entry.user.is_some() && entry.hostname.is_some() {
    warnings.push(format!(
      "no `IdentityFile`; provide `[auth.{}].password` in channel config when using this host",
      entry.host
    ));
    success_reasons.push("authentication can be supplied by channel config password".to_string());
  }

  let mut proxy_jump = entry.proxy_jump.clone();
  if proxy_jump.is_empty()
    && let Some(proxy_command) = entry.proxy_command.as_deref()
  {
    match parse_proxy_command_to_alias(proxy_command) {
      Some(alias) => proxy_jump.push(alias),
      None => {
        failure_reasons.push(format!(
          "`ProxyCommand {proxy_command}` is unsupported; use `ProxyJump <alias>` or `ProxyCommand ssh <alias> -W %h:%p`"
        ));
      }
    }
  }

  for token in &proxy_jump {
    if token.contains('@') || token.contains(':') {
      failure_reasons.push(format!(
        "ProxyJump '{token}' is a raw target; define a `Host <alias>` block and reference that alias"
      ));
      continue;
    }

    let Some(jump_entry) = by_alias.get(token.as_str()).copied() else {
      failure_reasons.push(format!(
        "ProxyJump '{token}' references no `Host {token}` block"
      ));
      continue;
    };

    if jump_entry.hostname.is_none() {
      failure_reasons.push(format!("ProxyJump alias '{token}' is missing `HostName`"));
    }
    if jump_entry.user.is_none() {
      failure_reasons.push(format!("ProxyJump alias '{token}' is missing `User`"));
    }
    if jump_entry.identity_file.is_none() {
      match default_key_count {
        0 => failure_reasons.push(format!(
          "ProxyJump alias '{token}' has no `IdentityFile` and no default key is available"
        )),
        1 => {}
        _ => failure_reasons.push(format!(
          "ProxyJump alias '{token}' has no `IdentityFile` and multiple default keys are available"
        )),
      }
    }
  }

  if failure_reasons.is_empty() {
    if proxy_jump.is_empty() {
      success_reasons.push("no ProxyJump chain".to_string());
    } else {
      success_reasons.push("ProxyJump chain uses resolvable aliases".to_string());
    }
  }

  let (status, reasons) = if failure_reasons.is_empty() {
    (HostSupportStatus::Supported, success_reasons)
  } else {
    (HostSupportStatus::Unsupported, failure_reasons)
  };

  HostSupportReport {
    alias: entry.host.clone(),
    hostname: entry.hostname.clone(),
    status,
    reasons,
    warnings,
  }
}
