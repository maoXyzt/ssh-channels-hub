use serde_json::Value;
use ssh_channels_hub::host_check::{HostSupportStatus, analyze_hosts};
use ssh_channels_hub::ssh_config::parse_ssh_config;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn write_ssh_config(content: &str) -> PathBuf {
  let unique = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap()
    .as_nanos();
  let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
  let path = std::env::temp_dir().join(format!(
    "ssh-channels-hub-hosts-{}-{count}-{unique}.config",
    std::process::id()
  ));
  fs::write(&path, content).expect("write temp ssh config");
  path
}

fn analyze(content: &str) -> Vec<ssh_channels_hub::host_check::HostSupportReport> {
  let path = write_ssh_config(content);
  let entries = parse_ssh_config(&path).expect("parse ssh config");
  analyze_hosts(&entries)
}

#[test]
fn complete_host_is_supported() {
  let reports = analyze(
    r#"
Host app
    HostName app.example.com
    User deploy
    IdentityFile ~/.ssh/id_ed25519
"#,
  );

  assert_eq!(reports.len(), 1);
  assert_eq!(reports[0].alias, "app");
  assert_eq!(reports[0].hostname.as_deref(), Some("app.example.com"));
  assert_eq!(reports[0].status, HostSupportStatus::Supported);
  assert!(reports[0].warnings.is_empty());
}

#[test]
fn host_missing_user_is_unsupported_with_reason() {
  let reports = analyze(
    r#"
Host app
    HostName app.example.com
    IdentityFile ~/.ssh/id_ed25519
"#,
  );

  assert_eq!(reports[0].status, HostSupportStatus::Unsupported);
  assert!(
    reports[0]
      .reasons
      .iter()
      .any(|reason| reason.contains("missing `User`")),
    "expected missing User reason, got {:?}",
    reports[0].reasons
  );
}

#[test]
fn target_without_identity_file_is_supported_with_password_warning() {
  let reports = analyze(
    r#"
Host app
    HostName app.example.com
    User deploy
"#,
  );

  assert_eq!(reports[0].status, HostSupportStatus::Supported);
  assert!(
    reports[0]
      .warnings
      .iter()
      .any(|warning| warning.contains("[auth.app].password")),
    "expected password warning, got {:?}",
    reports[0].warnings
  );
}

#[test]
fn raw_proxy_jump_target_is_unsupported() {
  let reports = analyze(
    r#"
Host app
    HostName app.example.com
    User deploy
    IdentityFile ~/.ssh/id_ed25519
    ProxyJump jumpuser@bastion.example.com:2222
"#,
  );

  assert_eq!(reports[0].status, HostSupportStatus::Unsupported);
  assert!(
    reports[0]
      .reasons
      .iter()
      .any(|reason| reason.contains("raw target")),
    "expected raw target reason, got {:?}",
    reports[0].reasons
  );
}

#[test]
fn missing_proxy_jump_alias_is_unsupported() {
  let reports = analyze(
    r#"
Host app
    HostName app.example.com
    User deploy
    IdentityFile ~/.ssh/id_ed25519
    ProxyJump missing-bastion
"#,
  );

  assert_eq!(reports[0].status, HostSupportStatus::Unsupported);
  assert!(
    reports[0]
      .reasons
      .iter()
      .any(|reason| reason.contains("no `Host missing-bastion`")),
    "expected missing alias reason, got {:?}",
    reports[0].reasons
  );
}

#[test]
fn hosts_format_json_outputs_complete_fields() {
  let path = write_ssh_config(
    r#"
Host app
    HostName app.example.com
    User deploy
    IdentityFile ~/.ssh/id_ed25519
"#,
  );

  let output = Command::new(env!("CARGO_BIN_EXE_ssh-channels-hub"))
    .args(["hosts", "--ssh-config"])
    .arg(path)
    .args(["--format", "json", "--no-color"])
    .output()
    .expect("run hosts command");

  assert!(
    output.status.success(),
    "hosts command failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );

  let parsed: Value = serde_json::from_slice(&output.stdout).expect("parse JSON output");
  let first = parsed.as_array().unwrap().first().unwrap();
  for field in ["alias", "hostname", "status", "reasons", "warnings"] {
    assert!(first.get(field).is_some(), "missing field {field}");
  }
  assert_eq!(first["alias"], "app");
  assert_eq!(first["status"], "supported");
}
