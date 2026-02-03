# SSH Channels Hub

A CLI application to create and manage SSH channels (port forwarding over SSH).

Cross-platform (Windows, Linux), written in Rust.

## Features

- **Port forwarding**: Listen on local ports and forward traffic to remote hosts via SSH tunnels (direct-tcpip).
- **Hosts + channels**: Define SSH hosts once, then reference them in channel configs (hostname, ports, dest_host, listen_host).
- **Automatic reconnection**: Reconnect with configurable backoff when the connection is lost.
- **Background service**: Run in foreground or background; CLI to start/stop/restart and show status.
- **Config validation**: Validate config file; generate config from `~/.ssh/config`.

## Usage

### Installation

Build from source:

```bash
cargo build --release
```

The binary will be at `target/release/ssh-channels-hub` (or `ssh-channels-hub.exe` on Windows).

### Configuration

1. **Config file location** (default):
   - **Linux/macOS**: `~/.config/ssh-channels-hub/config.toml`
   - **Windows**: `%APPDATA%\ssh-channels-hub\config.toml`

2. **Copy the example config**:

   ```bash
   mkdir -p ~/.config/ssh-channels-hub
   cp configs.example.toml ~/.config/ssh-channels-hub/config.toml
   ```

3. Edit the file with your hosts and channels. See [Configuration](docs/configuration.md) for details.

### Basic Commands

#### Start the service

Foreground (for testing):

```bash
ssh-channels-hub start --foreground
```

Background:

```bash
ssh-channels-hub start
```

Custom config:

```bash
ssh-channels-hub start --config /path/to/config.toml
```

Debug logging:

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

#### Test channels

Test that configured channels are reachable (connect to local ports):

```bash
ssh-channels-hub test
ssh-channels-hub test --config /path/to/config.toml
```

#### Validate configuration

```bash
ssh-channels-hub validate
ssh-channels-hub validate --config /path/to/config.toml
```

#### Generate configuration from SSH config

Generate a config from `~/.ssh/config`:

```bash
ssh-channels-hub generate
ssh-channels-hub generate --ssh-config /path/to/ssh_config --output /path/to/config.toml
```

The generated file contains `[[hosts]]` entries. Add `[[channels]]` sections (hostname, ports, optional dest_host / listen_host) for port forwarding.

### Configuration format (summary)

- **Hosts** (`[[hosts]]`): `name`, `host`, `port`, `username`, `auth` (key or password).
- **Channels** (`[[channels]]`): `name`, `hostname` (must match a host), `ports` in form `"local:dest"` (e.g. `"80:3923"` = listen on local port 80, forward to remote port 3923).
- **Optional per channel**: `dest_host` (default `127.0.0.1`), `listen_host` (default `127.0.0.1`; use `0.0.0.0` to accept connections from any interface).

### Configuration examples

#### Port forwarding (local 8080 → remote 80)

```toml
[[hosts]]
name = "web-server"
host = "example.com"
port = 22
username = "user"

[hosts.auth]
type = "key"
key_path = "~/.ssh/id_rsa"

[[channels]]
name = "web-tunnel"
hostname = "web-server"
ports = "8080:80"
dest_host = "127.0.0.1"
# listen_host = "127.0.0.1"   # default; use "0.0.0.0" to allow other machines to connect
```

#### Port forwarding (listen on all interfaces)

```toml
[[channels]]
name = "db-tunnel"
hostname = "db-server"
ports = "3306:3306"
dest_host = "127.0.0.1"
listen_host = "0.0.0.0"
```

### Common use cases

1. **Secure DB access**: Forward local 3306 to remote MySQL (e.g. `ports = "3306:3306"`).
2. **Remote web service**: Forward local 8080 to remote 80 (e.g. `ports = "8080:80"`).
3. **Multiple tunnels**: Define several channels; all start together and reconnect independently.
4. **Expose to LAN**: Set `listen_host = "0.0.0.0"` so other machines can use the tunnel (consider firewall and security).

### Troubleshooting

- **Connection fails**: Check SSH credentials and network; try `ssh user@host` manually.
- **Port in use**: Change `ports` (e.g. use 18080 instead of 80) or stop the app using the port.
- **Bind 80 on Windows**: Often requires running as Administrator.
- **Config errors**: Run `ssh-channels-hub validate`.
- **Debug**: Use `ssh-channels-hub start --debug`.
- **Key permissions**: Ensure SSH key file has correct permissions (e.g. 600).

More details: [Documentation](docs/README.md), [How to use](docs/HowToUse.md), [Configuration](docs/configuration.md).
