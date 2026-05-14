// Integration tests for AppConfig::build_channels: resolve `[[channels]]`
// against ~/.ssh/config and apply [auth.<alias>] overrides.

use ssh_channels_hub::config::{AppConfig, AuthConfig};
use std::path::PathBuf;

/// SSH config fixture shared by these tests:
///   myserver  -> example.com:22, user myuser, IdentityFile ~/.ssh/id_rsa
///   myserver2 -> example2.com:2222, user user2, NO IdentityFile
///   myserver3 -> example3.com:22, user admin, IdentityFile ~/.ssh/id_ed25519
fn test_ssh_config_path() -> PathBuf {
  PathBuf::from("tests/test_ssh_config")
}

fn parse(toml: &str) -> AppConfig {
  let mut config: AppConfig = toml::from_str(toml).expect("parse TOML");
  // Point at the test fixture rather than the user's real ~/.ssh/config.
  config.ssh_config = Some(test_ssh_config_path());
  config
}

#[test]
fn channel_uses_identity_file_from_ssh_config() {
  let config = parse(
    r#"
[[channels]]
name = "rsa-tunnel"
hostname = "myserver"
ports = "8080:80"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  assert_eq!(channels.len(), 1);

  let ch = &channels[0];
  assert_eq!(ch.host, "example.com");
  assert_eq!(ch.port, 22);
  assert_eq!(ch.username, "myuser");
  match &ch.auth {
    AuthConfig::Key {
      key_path,
      passphrase,
    } => {
      assert!(key_path.to_string_lossy().ends_with("id_rsa"));
      assert!(passphrase.is_none());
    }
    AuthConfig::Password { .. } => panic!("expected key auth from SSH config IdentityFile"),
  }
}

#[test]
fn auth_override_password_wins_over_identity_file() {
  let config = parse(
    r#"
[[channels]]
name = "pw-tunnel"
hostname = "myserver"
ports = "8080:80"

[auth.myserver]
password = "secret"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  match &channels[0].auth {
    AuthConfig::Password { password } => assert_eq!(password, "secret"),
    _ => panic!("password override should take precedence over SSH config IdentityFile"),
  }
}

#[test]
fn auth_override_passphrase_attaches_to_ssh_config_key() {
  let config = parse(
    r#"
[[channels]]
name = "ed25519-tunnel"
hostname = "myserver3"
ports = "9090:90"

[auth.myserver3]
passphrase = "open-sesame"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  match &channels[0].auth {
    AuthConfig::Key {
      key_path,
      passphrase,
    } => {
      assert!(key_path.to_string_lossy().ends_with("id_ed25519"));
      assert_eq!(passphrase.as_deref(), Some("open-sesame"));
    }
    _ => panic!("expected key auth with passphrase override"),
  }
}

#[test]
fn password_required_when_ssh_config_has_no_identity_file() {
  // myserver2 has no IdentityFile and no [auth.myserver2] override → must error.
  let config = parse(
    r#"
[[channels]]
name = "broken"
hostname = "myserver2"
ports = "8080:80"
"#,
  );

  let err = config
    .build_channels()
    .expect_err("build should fail without IdentityFile or password override");
  let msg = err.to_string();
  assert!(
    msg.contains("myserver2"),
    "error should name the offending alias, got: {msg}"
  );
}

#[test]
fn password_override_satisfies_host_without_identity_file() {
  let config = parse(
    r#"
[[channels]]
name = "pw-only"
hostname = "myserver2"
ports = "8080:80"

[auth.myserver2]
password = "pw"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  assert_eq!(channels[0].host, "example2.com");
  assert_eq!(channels[0].port, 2222); // Non-default port from SSH config
  match &channels[0].auth {
    AuthConfig::Password { password } => assert_eq!(password, "pw"),
    _ => panic!("expected password auth from override"),
  }
}

#[test]
fn unknown_host_alias_is_rejected() {
  let config = parse(
    r#"
[[channels]]
name = "ghost"
hostname = "does-not-exist"
ports = "8080:80"
"#,
  );

  let err = config
    .build_channels()
    .expect_err("unknown alias must fail");
  let msg = err.to_string();
  assert!(
    msg.contains("does-not-exist"),
    "error should name the missing alias, got: {msg}"
  );
}

#[test]
fn multiple_channels_can_share_one_alias() {
  let config = parse(
    r#"
[[channels]]
name = "web"
hostname = "myserver"
ports = "8080:80"

[[channels]]
name = "db"
hostname = "myserver"
ports = "3306:3306"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  assert_eq!(channels.len(), 2);
  assert_eq!(channels[0].host, channels[1].host);
  assert_eq!(channels[0].username, channels[1].username);
}
