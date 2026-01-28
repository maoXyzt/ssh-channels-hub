# SSH Channels Hub

An CLI application to create and manage SSH channels.

Cross-platform (Windows, Linux), written in Rust.

## Features

- Read local configuration file to get the list of channels to open.
- Open SSH channels to remote servers according to the configuration file.
- If connection is lost, try to reconnect.
- If channel is closed, try to re-open it.
- Runs on the background, as a service.
- Has a CLI interface to start/stop/restart the service, and show the status of the service.

## Usage

### Installation

Build from source:

```bash
cargo build --release
```

The binary will be located at `target/release/ssh-channels-hub`.

### Configuration

1. Create a configuration file. The default location is:
   - **Linux/macOS**: `~/.config/ssh-channels-hub/config.toml`
   - **Windows**: `%APPDATA%\ssh-channels-hub\config.toml`

2. Copy the example configuration:

   ```bash
   mkdir -p ~/.config/ssh-channels-hub
   cp configs/config.example.toml ~/.config/ssh-channels-hub/config.toml
   ```

3. Edit the configuration file with your SSH channel settings. See [Configuration Documentation](docs/configuration.md) for details.

### Basic Commands

#### Start the service

Start the service in foreground mode (for testing):

```bash
ssh-channels-hub start --foreground
```

Start the service in background (daemon mode - future feature):

```bash
ssh-channels-hub start
```

Use a custom configuration file:

```bash
ssh-channels-hub start --config /path/to/config.toml
```

Enable debug logging:

```bash
ssh-channels-hub start --debug
```

#### Stop the service

```bash
ssh-channels-hub stop
```

#### Restart the service

```bash
ssh-channels-hub restart
```

#### Check service status

```bash
ssh-channels-hub status
```

#### Validate configuration

Validate the default configuration file:

```bash
ssh-channels-hub validate
```

Validate a specific configuration file:

```bash
ssh-channels-hub validate --config /path/to/config.toml
```

#### Generate configuration from SSH config

Generate a `configs.toml` file from your `~/.ssh/config`:

```bash
ssh-channels-hub generate
```

Use a custom SSH config file:

```bash
ssh-channels-hub generate --ssh-config /path/to/ssh_config
```

Specify output file:

```bash
ssh-channels-hub generate --output /path/to/configs.toml
```

**Note**: The generated configuration will:

- Extract `name`, `host`, `port`, and `username` from SSH config entries
- Set `channel_type` to `direct-tcpip` by default
- Use key authentication if `IdentityFile` is specified in SSH config
- Use password authentication with placeholder `CHANGE_ME` if no `IdentityFile` is found (you'll need to update the password manually)

### Configuration Examples

#### Basic Session Channel

```toml
[[channels]]
name = "web-server"
host = "example.com"
port = 22
username = "user"
channel_type = "session"

[channels.auth]
type = "key"
key_path = "~/.ssh/id_rsa"
```

#### Execute Command

```toml
[[channels]]
name = "log-monitor"
host = "server.example.com"
port = 22
username = "monitor"
channel_type = "session"

[channels.auth]
type = "password"
password = "your-password"

[channels.params]
command = "tail -f /var/log/application.log"
```

#### Port Forwarding (Direct-TCPIP)

```toml
[[channels]]
name = "db-tunnel"
host = "db.example.com"
port = 22
username = "user"
channel_type = "direct-tcpip"

[channels.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[channels.params]
destination_host = "localhost"
destination_port = 3306
```

### Common Use Cases

1. **Monitor remote logs**: Configure a session channel with a command to tail logs
2. **Port forwarding**: Set up SSH tunnels for secure database access
3. **Multiple connections**: Manage multiple SSH connections from a single configuration
4. **Automatic reconnection**: Keep connections alive with automatic reconnection on failure

### Troubleshooting

- **Connection fails**: Check your SSH credentials and network connectivity
- **Configuration errors**: Use `validate` command to check your configuration file
- **Debug issues**: Use `--debug` flag to see detailed logs
- **Permission errors**: Ensure your SSH key file has correct permissions (600)

For more information, see the [Documentation](docs/README.md).
