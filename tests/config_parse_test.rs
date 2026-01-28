// Test to verify TOML parsing supports different auth per channel

use ssh_channels_hub::config::AppConfig;

#[test]
fn test_multiple_channels_different_auth() {
    let toml_content = r#"
[reconnection]
max_retries = 0
initial_delay_secs = 1
max_delay_secs = 30
use_exponential_backoff = true

# Channel 1: Password authentication
[[channels]]
name = "test-password"
host = "example.com"
port = 22
username = "user1"
channel_type = "session"

[channels.auth]
type = "password"
password = "test-password-123"

# Channel 2: Key authentication with default key
[[channels]]
name = "test-key-default"
host = "example.com"
port = 22
username = "user2"
channel_type = "session"

[channels.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

# Channel 3: Key authentication with different key
[[channels]]
name = "test-key-custom"
host = "example.com"
port = 22
username = "user3"
channel_type = "session"

[channels.auth]
type = "key"
key_path = "~/.ssh/custom_key"
passphrase = "custom-passphrase"
"#;

    let config: AppConfig =
        toml::from_str(toml_content).expect("Failed to parse TOML configuration");

    // Verify we have 3 channels
    assert_eq!(config.channels.len(), 3);

    // Verify Channel 1: Password authentication
    let ch1 = &config.channels[0];
    assert_eq!(ch1.name, "test-password");
    assert_eq!(ch1.username, "user1");
    match &ch1.auth {
        ssh_channels_hub::config::AuthConfig::Password { password } => {
            assert_eq!(password, "test-password-123");
        }
        _ => panic!("Channel 1 should use password authentication"),
    }

    // Verify Channel 2: Key authentication with default key
    let ch2 = &config.channels[1];
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

    // Verify Channel 3: Key authentication with different key
    let ch3 = &config.channels[2];
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

    // Verify channels 2 and 3 use different keys
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

    // Only run this test if the test config file exists
    if test_config_path.exists() {
        let config = AppConfig::from_file(&test_config_path)
            .expect("Failed to load test configuration file");

        assert!(
            !config.channels.is_empty(),
            "Config should have at least one channel"
        );

        // Verify each channel has its own auth configuration
        for channel in &config.channels {
            match &channel.auth {
                ssh_channels_hub::config::AuthConfig::Password { .. } => {
                    // Password auth is valid
                }
                ssh_channels_hub::config::AuthConfig::Key { .. } => {
                    // Key auth is valid
                }
            }
        }
    }
}
