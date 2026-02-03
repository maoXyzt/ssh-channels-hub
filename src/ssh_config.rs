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

/// Parse SSH config content
fn parse_ssh_config_content(content: &str) -> Result<Vec<SshConfigEntry>> {
    let mut entries = Vec::new();
    let mut current_host: Option<String> = None;
    let mut current_config: HashMap<String, String> = HashMap::new();
    let mut defaults = SshConfigDefaults::default();
    let mut is_default_host = false;

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle Host directive (starts a new entry)
        if line.starts_with("Host ") {
            // Save previous entry if exists
            if let Some(host) = current_host.take() {
                if !is_default_host {
                    if let Some(entry) = build_entry(&host, &current_config, &defaults) {
                        entries.push(entry);
                    }
                } else {
                    // This was Host "*", save as defaults
                    defaults = extract_defaults(&current_config);
                }
            }
            current_config.clear();
            is_default_host = false;

            // Extract host name(s) - can be space-separated or wildcards
            let hosts = line[4..].trim();
            // For simplicity, we'll use the first host name
            // In real SSH config, multiple hosts can share the same config
            let host = hosts.split_whitespace().next().unwrap_or("").to_string();

            if host == "*" {
                // This is the default host entry
                is_default_host = true;
                current_host = Some(host);
            } else if !host.is_empty() {
                current_host = Some(host);
            }
        } else if current_host.is_some() {
            // Parse other directives
            if let Some((key, value)) = parse_directive(line) {
                current_config.insert(key.to_lowercase(), value);
            }
        }
    }

    // Save last entry
    if let Some(host) = current_host {
        if !is_default_host {
            if let Some(entry) = build_entry(&host, &current_config, &defaults) {
                entries.push(entry);
            }
        } else {
            // This was Host "*", save as defaults (for potential future use)
            // Note: This won't affect already processed entries, which matches SSH config behavior
            let _ = extract_defaults(&current_config);
        }
    }

    Ok(entries)
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

/// Parse a directive line (e.g., "HostName example.com")
fn parse_directive(line: &str) -> Option<(&str, String)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 {
        let key = parts[0];
        let value = parts[1..].join(" "); // Handle values with spaces
        Some((key, value))
    } else {
        None
    }
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
        .or_else(|| defaults.port);

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
}
