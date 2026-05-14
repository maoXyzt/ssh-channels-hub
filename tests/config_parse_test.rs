// Integration tests for AppConfig::build_channels: resolve `[[channels]]`
// against ~/.ssh/config and apply [auth.<alias>] overrides.

use ssh_channels_hub::config::{
  AppConfig, AuthConfig, ChannelTypeParams, ConnectionConfig, Direction,
};
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

// --- SSH-config integration ---

#[test]
fn channel_uses_identity_file_from_ssh_config() {
  let config = parse(
    r#"
[[channels]]
name = "rsa-tunnel"
hostname = "myserver"
direction = "local->remote"
local = "8080"
remote = "80"
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
direction = "local->remote"
local = "8080"
remote = "80"

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
direction = "local->remote"
local = "9090"
remote = "90"

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
direction = "local->remote"
local = "8080"
remote = "80"
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
direction = "local->remote"
local = "8080"
remote = "80"

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
direction = "local->remote"
local = "8080"
remote = "80"
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
direction = "local->remote"
local = "8080"
remote = "80"

[[channels]]
name = "db"
hostname = "myserver"
direction = "local->remote"
local = "3306"
remote = "3306"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  assert_eq!(channels.len(), 2);
  assert_eq!(channels[0].host, channels[1].host);
  assert_eq!(channels[0].username, channels[1].username);
}

// --- Direction / Endpoint coverage ---

#[test]
fn direction_local_to_remote_maps_to_direct_tcpip() {
  let config = parse(
    r#"
[[channels]]
name = "db"
hostname = "myserver"
direction = "local->remote"
local = "127.0.0.1:3306"
remote = "10.0.0.5:5432"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  match &channels[0].params {
    ChannelTypeParams::DirectTcpIp {
      listen_host,
      local_port,
      dest_host,
      dest_port,
    } => {
      assert_eq!(listen_host, "127.0.0.1");
      assert_eq!(*local_port, 3306);
      assert_eq!(dest_host, "10.0.0.5");
      assert_eq!(*dest_port, 5432);
    }
    other => panic!("expected DirectTcpIp, got {:?}", other),
  }
}

#[test]
fn direction_remote_to_local_maps_to_forwarded_tcpip() {
  let config = parse(
    r#"
[[channels]]
name = "expose"
hostname = "myserver"
direction = "remote->local"
remote = "0.0.0.0:8022"
local = "127.0.0.1:80"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  match &channels[0].params {
    ChannelTypeParams::ForwardedTcpIp {
      remote_bind_host,
      remote_bind_port,
      local_connect_host,
      local_connect_port,
    } => {
      assert_eq!(remote_bind_host, "0.0.0.0");
      assert_eq!(*remote_bind_port, 8022);
      assert_eq!(local_connect_host, "127.0.0.1");
      assert_eq!(*local_connect_port, 80);
    }
    other => panic!("expected ForwardedTcpIp, got {:?}", other),
  }
}

#[test]
fn endpoint_defaults_to_loopback_when_port_only() {
  let config = parse(
    r#"
[[channels]]
name = "db"
hostname = "myserver"
direction = "local->remote"
local = "3306"
remote = "3306"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  match &channels[0].params {
    ChannelTypeParams::DirectTcpIp {
      listen_host,
      dest_host,
      ..
    } => {
      assert_eq!(listen_host, "127.0.0.1");
      assert_eq!(dest_host, "127.0.0.1");
    }
    _ => panic!("expected DirectTcpIp"),
  }
}

#[test]
fn endpoint_accepts_explicit_host_to_listen_on_all_interfaces() {
  let config = parse(
    r#"
[[channels]]
name = "shared-db"
hostname = "myserver"
direction = "local->remote"
local = "0.0.0.0:3306"
remote = "3306"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  match &channels[0].params {
    ChannelTypeParams::DirectTcpIp { listen_host, .. } => {
      assert_eq!(listen_host, "0.0.0.0");
    }
    _ => panic!("expected DirectTcpIp"),
  }
}

#[test]
fn local_listen_port_only_reported_for_local_to_remote() {
  let l2r: ConnectionConfig = toml::from_str(
    r#"
name = "out"
hostname = "myserver"
direction = "local->remote"
local = "3306"
remote = "3306"
"#,
  )
  .unwrap();
  assert_eq!(l2r.local_listen_port(), Some(3306));
  assert_eq!(l2r.direction, Direction::LocalToRemote);

  let r2l: ConnectionConfig = toml::from_str(
    r#"
name = "in"
hostname = "myserver"
direction = "remote->local"
remote = "8022"
local = "80"
"#,
  )
  .unwrap();
  assert_eq!(r2l.local_listen_port(), None);
  assert_eq!(r2l.direction, Direction::RemoteToLocal);
}

#[test]
fn unknown_direction_is_rejected_with_clear_error() {
  let err = toml::from_str::<AppConfig>(
    r#"
[[channels]]
name = "bad"
hostname = "myserver"
direction = "outbound"
local = "80"
remote = "80"
"#,
  )
  .expect_err("invalid direction must fail at deserialization");
  let msg = err.to_string();
  assert!(
    msg.contains("local->remote") && msg.contains("remote->local"),
    "error should mention valid choices, got: {msg}"
  );
}

#[test]
fn unknown_fields_are_rejected() {
  let err = toml::from_str::<AppConfig>(
    r#"
[[channels]]
name = "bogus"
hostname = "myserver"
direction = "local->remote"
local = "8080"
remote = "80"
extra_field = "nope"
"#,
  )
  .expect_err("unknown fields must be rejected via deny_unknown_fields");
  assert!(err.to_string().contains("extra_field"));
}

#[test]
fn missing_direction_is_rejected() {
  let err = toml::from_str::<AppConfig>(
    r#"
[[channels]]
name = "no-direction"
hostname = "myserver"
local = "80"
remote = "80"
"#,
  )
  .expect_err("missing `direction` must be rejected");
  let msg = err.to_string();
  assert!(
    msg.contains("direction"),
    "error should name the missing field, got: {msg}"
  );
}

#[test]
fn missing_local_or_remote_is_rejected() {
  let err = toml::from_str::<AppConfig>(
    r#"
[[channels]]
name = "no-remote"
hostname = "myserver"
direction = "local->remote"
local = "80"
"#,
  )
  .expect_err("missing `remote` must be rejected");
  assert!(err.to_string().contains("remote"));

  let err = toml::from_str::<AppConfig>(
    r#"
[[channels]]
name = "no-local"
hostname = "myserver"
direction = "local->remote"
remote = "80"
"#,
  )
  .expect_err("missing `local` must be rejected");
  assert!(err.to_string().contains("local"));
}

#[test]
fn bad_endpoint_format_is_rejected_with_useful_message() {
  let err = toml::from_str::<AppConfig>(
    r#"
[[channels]]
name = "bad-port"
hostname = "myserver"
direction = "local->remote"
local = "127.0.0.1:not-a-port"
remote = "80"
"#,
  )
  .expect_err("bad port must be rejected");
  let msg = err.to_string();
  assert!(
    msg.contains("port") || msg.contains("bad port"),
    "error should mention the port problem, got: {msg}"
  );
}

#[test]
fn ipv6_bracketed_endpoint_works() {
  let config = parse(
    r#"
[[channels]]
name = "v6"
hostname = "myserver"
direction = "local->remote"
local = "[::1]:3306"
remote = "127.0.0.1:3306"
"#,
  );

  let channels = config.build_channels().expect("build_channels");
  match &channels[0].params {
    ChannelTypeParams::DirectTcpIp { listen_host, .. } => assert_eq!(listen_host, "::1"),
    _ => panic!("expected DirectTcpIp"),
  }
}
