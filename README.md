# SSH Channels Hub

A CLI application to create and manage SSH channels (port forwarding over SSH).

Cross-platform (Windows, Linux), written in Rust.

## Features

- **Port forwarding**: Listen on local ports and forward traffic to remote hosts via SSH tunnels (direct-tcpip).
- **Hosts + channels**: Define SSH hosts once, then reference them in channel configs (hostname, ports, dest_host, listen_host).
- **Automatic reconnection**: Reconnect with configurable backoff when the connection is lost.
- **Foreground / daemon**: Default `start` runs in foreground; `start -D` runs as daemon (detached). Stop and restart use IPC so the process exits cleanly.
- **Config validation**: Validate config file; generate config from `~/.ssh/config`.

## Usage

### Installation

Build from source:

```bash
cargo build --release
```

The binary will be at `target/release/ssh-channels-hub` (or `ssh-channels-hub.exe` on Windows).

### Configuration

1. **Config file location** (default; first existing wins):
   - **Current directory**: `./configs.toml`
   - **Linux/macOS**: `~/.config/ssh-channels-hub/config.toml`
   - **Windows**: `%APPDATA%\ssh-channels-hub\config.toml`

2. **Copy the example config**:

   ```bash
   cp configs.example.toml configs.toml
   # or into platform dir:
   mkdir -p ~/.config/ssh-channels-hub
   cp configs.example.toml ~/.config/ssh-channels-hub/config.toml
   ```

3. Edit the file with your hosts and channels. Use `--config /path/to/config.toml` to override. See [Configuration](docs/configuration.md) for details.

### Basic Commands

#### Start the service

**Foreground** (default; press Ctrl+C to stop):

```bash
ssh-channels-hub start
```

**Daemon** (background; spawns detached process):

```bash
ssh-channels-hub start -D
# or
ssh-channels-hub start --daemon
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

Sends a stop signal via IPC so the service exits gracefully, then removes run files (`.pid`, `.port`). Use the same `--config` as start if you use a non-default config.

```bash
ssh-channels-hub stop
ssh-channels-hub stop --config /path/to/config.toml
```

#### Restart the service

Stops the running service (via IPC if running), then starts it again as a **daemon**. Use the same `--config` as the running service.

```bash
ssh-channels-hub restart
```

#### Check service status

Connects to the running process via IPC and shows state (with emoji), active channels, config path, PID, and channel list. If the service is not running, shows Stopped and channel list from config.

```bash
ssh-channels-hub status
ssh-channels-hub status --config /path/to/config.toml
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
- **Channels** (`[[channels]]`): `name`, `hostname` (must match a host), `ports`. Optional: `channel_type`, `dest_host`, `listen_host`.
  - **Local forward** (default, like `ssh -L`): `ports = "local:dest"` (e.g. `"80:3923"` = listen local 80 → remote 3923).
  - **Remote forward** (like `ssh -R`): `channel_type = "forwarded-tcpip"`, `ports = "remote:local"` (e.g. `"8022:80"` = bind 8022 on server → connect to local 127.0.0.1:80).
- **Optional per channel**: `dest_host` (default `127.0.0.1`), `listen_host` (default `127.0.0.1`; use `0.0.0.0` for all interfaces; local forward only).

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

#### Remote port forwarding (ssh -R style)

Expose a local service on the SSH server: bind a port on the server and bridge connections to a local address.

```toml
[[channels]]
name = "expose-local-web"
channel_type = "forwarded-tcpip"
hostname = "web-server"
ports = "8022:80"           # remote port 8022 -> local 127.0.0.1:80
dest_host = "127.0.0.1"    # local host to connect to (default)
```

### Common use cases

1. **Secure DB access**: Forward local 3306 to remote MySQL (e.g. `ports = "3306:3306"`).
2. **Remote web service**: Forward local 8080 to remote 80 (e.g. `ports = "8080:80"`).
3. **Remote forward (ssh -R)**: Expose local service on server (e.g. `channel_type = "forwarded-tcpip"`, `ports = "8022:80"`).
4. **Multiple tunnels**: Define several channels; all start together and reconnect independently.
5. **Expose to LAN**: Set `listen_host = "0.0.0.0"` so other machines can use the tunnel (consider firewall and security).

### Troubleshooting

- **Connection fails**: Check SSH credentials and network; try `ssh user@host` manually.
- **Port in use**: Change `ports` (e.g. use 18080 instead of 80) or stop the app using the port.
- **Bind 80 on Windows**: Often requires running as Administrator.
- **Config errors**: Run `ssh-channels-hub validate`.
- **Debug**: Use `ssh-channels-hub start --debug` or `--debug` with any command.
- **Key permissions**: Ensure SSH key file has correct permissions (e.g. 600).

More details: [Documentation](docs/README.md), [How to use](docs/HowToUse.md), [Configuration](docs/configuration.md).
