// Test to verify TOML parsing supports different auth per channel (via host reference)

use ssh_channels_hub::config::AppConfig;

#[test]
fn test_multiple_channels_different_auth() {
    let toml_content = r#"
[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true

[[hosts]]
name = "host-password"
host = "example.com"
port = 22
username = "user1"

[hosts.auth]
type = "password"
password = "test-password-123"

[[hosts]]
name = "host-key-default"
host = "example.com"
port = 22
username = "user2"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[[hosts]]
name = "host-key-custom"
host = "example.com"
port = 22
username = "user3"

[hosts.auth]
type = "key"
key_path = "~/.ssh/custom_key"
passphrase = "custom-passphrase"

[[channels]]
name = "test-password"
hostname = "host-password"
ports = "8080:80"

[[channels]]
name = "test-key-default"
hostname = "host-key-default"
ports = "8081:80"

[[channels]]
name = "test-key-custom"
hostname = "host-key-custom"
ports = "8082:80"
"#;

    let config: AppConfig =
        toml::from_str(toml_content).expect("Failed to parse TOML configuration");

    assert_eq!(config.hosts.len(), 3);
    assert_eq!(config.channels.len(), 3);

    let channels = config.build_channels().expect("build_channels");

    // Channel 1: Password authentication (from host-password)
    let ch1 = &channels[0];
    assert_eq!(ch1.name, "test-password");
    assert_eq!(ch1.username, "user1");
    match &ch1.auth {
        ssh_channels_hub::config::AuthConfig::Password { password } => {
            assert_eq!(password, "test-password-123");
        }
        _ => panic!("Channel 1 should use password authentication"),
    }

    // Channel 2: Key authentication with default key
    let ch2 = &channels[1];
    assert_eq!(ch2.name, "test-key-default");
    assert_eq!(ch2.username, "user2");
    match &ch2.auth {
        ssh_channels_hub::config::AuthConfig::Key {
            key_path,
            passphrase,
        } => {
            assert_eq!(key_path.to_string_lossy(), "~/.ssh/id_rsa");
            assert!(passphrase.is_none());
        }
        _ => panic!("Channel 2 should use key authentication"),
    }

    // Channel 3: Key authentication with different key
    let ch3 = &channels[2];
    assert_eq!(ch3.name, "test-key-custom");
    assert_eq!(ch3.username, "user3");
    match &ch3.auth {
        ssh_channels_hub::config::AuthConfig::Key {
            key_path,
            passphrase,
        } => {
            assert_eq!(key_path.to_string_lossy(), "~/.ssh/custom_key");
            assert_eq!(passphrase.as_deref(), Some("custom-passphrase"));
        }
        _ => panic!("Channel 3 should use key authentication"),
    }

    if let (
        ssh_channels_hub::config::AuthConfig::Key { key_path: k2, .. },
        ssh_channels_hub::config::AuthConfig::Key { key_path: k3, .. },
    ) = (&ch2.auth, &ch3.auth)
    {
        assert_ne!(k2, k3, "Channels 2 and 3 should use different keys");
    }
}

#[test]
fn test_load_config_from_file() {
    use std::path::PathBuf;

    let test_config_path = PathBuf::from("tests/test_multi_auth.toml");

    if test_config_path.exists() {
        let config = AppConfig::from_file(&test_config_path)
            .expect("Failed to load test configuration file");

        assert!(
            !config.channels.is_empty(),
            "Config should have at least one channel"
        );

        let channels = config.build_channels().expect("build_channels");
        for channel in &channels {
            match &channel.auth {
                ssh_channels_hub::config::AuthConfig::Password { .. } => {}
                ssh_channels_hub::config::AuthConfig::Key { .. } => {}
            }
        }
    }
}
