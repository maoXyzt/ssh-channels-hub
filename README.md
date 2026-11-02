# SSH Channels Hub

> English | [中文](./README-zh.md)

Declarative SSH tunnels with auto-reconnect. Define your port forwards once in
TOML, start one service, and they all come up — reconnecting automatically when
the link drops.

Cross-platform (Linux, macOS, Windows). Written in Rust on top of [russh](https://docs.rs/russh).

## Why

Reach for this when `ssh -L 3306:127.0.0.1:3306 db.example.com` has grown into
*"I have five of those, my laptop sleeps, my Wi-Fi flakes, and I want them all
back when I open the lid."*

- **Declarative**: tunnels live in `config.toml`, not in shell history or terminal panes.
- **No host config duplication**: host info (`HostName` / `User` / `Port` / `IdentityFile`) is read straight from `~/.ssh/config` — you reference aliases.
- **ProxyJump aware**: chain through bastions defined in `~/.ssh/config` — using
alias-only references, public-key authentication, and strict `known_hosts`
checks for targets and jumps. See [docs/configuration.md §3.4](docs/configuration.md#34-where-host-information-comes-from).
- **Auto-reconnect**: compatible tunnels share one SSH session; a dropped route reconnects with jittered backoff without disturbing other routes.
- **Both directions in one schema**: local-to-remote (`ssh -L`) and remote-to-local (`ssh -R`).
- **Foreground or daemon**: `start` attaches to the terminal, `start -D` detaches; `stop` / `restart` / `status` talk to the running process via IPC.

## Quickstart

**1. Run or install**

Run directly with `uvx` (recommended, no installation required):

```bash
uvx ssh-channels-hub --help
```

Or install it with `pip` inside an activated virtual environment:

```bash
pip install ssh-channels-hub
ssh-channels-hub --help
```

The wheel installs the same `ssh-channels-hub` binary on Linux x86_64, macOS ARM64, and Windows x86_64; it does not run through Python.

If you already have a Rust/Cargo toolchain, you can also install with:

```bash
cargo binstall ssh-channels-hub          # requires cargo-binstall; installs a prebuilt binary
cargo install ssh-channels-hub --locked # build and install from source
```

For development, clone and build the source:

```bash
git clone https://github.com/maoXyzt/ssh-channels-hub.git
cd ssh-channels-hub
cargo build --release           # binary at target/release/ssh-channels-hub (or .exe on Windows)
```

**2. Have the host in** `~/.ssh/config`

```
Host my-db
  HostName db.example.com
  User myuser
  IdentityFile ~/.ssh/id_rsa
```

**3. Write** `config.toml` in the current directory:

```toml
[[channels]]
name      = "db"
hostname  = "my-db"             # alias from ~/.ssh/config
direction = "local->remote"     # ssh -L
local     = "3306"              # listen on 127.0.0.1:3306
remote    = "3306"              # server connects to 127.0.0.1:3306
```

**4. Run**

```bash
uvx ssh-channels-hub start      # no installation
# or, after pip or Cargo installation:
ssh-channels-hub start          # Ctrl+C to stop
# or, after cargo build:
./target/release/ssh-channels-hub start       # Linux/macOS
.\target\release\ssh-channels-hub.exe start  # Windows PowerShell
```

Now `mysql -h 127.0.0.1 -P 3306` goes through the tunnel.

> **Tip:** `ssh-channels-hub generate -o config.toml` scaffolds one commented-out `[[channels]]` block per alias in your SSH config — uncomment and fill in ports. Or `cp config.example.toml config.toml` for an annotated template.

## Configuration

`config.toml` is looked up in this order (first existing wins):


| Platform                               | Path                                                                         |
| -------------------------------------- | ---------------------------------------------------------------------------- |
| Current directory (always tried first) | `./config.toml`                                                              |
| Linux / macOS                          | `$XDG_CONFIG_HOME/ssh-channels-hub/config.toml` (`~/.config/...` when unset) |
| Windows                                | `%APPDATA%\ssh-channels-hub\config.toml`                                     |


`--config /path/to/file` overrides the lookup.

### Channel schema

```text
[[channels]]
name      = "string"                            # required, unique identifier
hostname  = "ssh-config-alias"                  # required; resolves via ~/.ssh/config
direction = "local->remote" | "remote->local"   # required
local     = "port" | "host:port" | "URL"        # required, this machine's side
remote    = "port" | "host:port" | "URL"        # required, the SSH server's side
mode      = "tcp" | "http-gateway"              # optional, default: tcp
```

`local` and `remote` always name the address on their respective side regardless of direction. Direction decides who listens:

- `local->remote` (≈ `ssh -L`): this machine listens on `local`; the server dials `remote` for each connection.
- `remote->local` (≈ `ssh -R`): the server binds `remote`; incoming traffic is bridged to `local` on this side.

Endpoints accept:

- `"3306"` → `127.0.0.1:3306` (bare port, host defaults to loopback)
- `"127.0.0.1:3306"` → explicit form
- `"0.0.0.0:8080"` → bind on every interface
- `"db.internal:5432"` → hostname and port
- `"[::1]:3306"` → IPv6
- `"https://api.internal/v1"` → `api.internal:443` (HTTP defaults to 80)

For `remote->local`, this machine resolves and dials the `local` hostname for every incoming connection. URL paths are not part of the TCP target. In the default `tcp` mode, HTTPS remains end-to-end: the daemon does not terminate TLS or rewrite Host, SNI, or request paths.

For a fixed HTTPS gateway, `http-gateway` instead accepts HTTP/1.1 from the remote client and makes the upstream HTTPS request with the correct Host and SNI. The remote listener must be loopback, so the plaintext HTTP leg exists only inside the SSH tunnel:

```toml
[[channels]]
name      = "llm-gateway"
hostname  = "edge-server"
direction = "remote->local"
remote    = "5443"
local     = "https://gateway.internal.example"
mode      = "http-gateway"
```

In `http-gateway` mode, `local = "host:port"` also selects a TLS upstream. An `https://` URL supports both HTTPS and WSS; its scheme specifies upstream TLS, while the request selects WebSocket via Upgrade.

The remote client then uses `http://127.0.0.1:5443/v1`, without an `/etc/hosts` entry or a certificate for `127.0.0.1`. Request paths, authorization headers, bodies, and streaming responses are preserved; WebSocket upgrades are supported (use `ws://127.0.0.1:5443/...` to reach the WSS upstream); CONNECT and other protocol upgrades are not supported. The server's `sshd_config` must use `GatewayPorts no` or `clientspecified`: `GatewayPorts yes` ignores the requested loopback address and can expose the plaintext listener on all interfaces.


Choose the mode by who should handle TLS:

| Need | Mode |
| --- | --- |
| Database/other TCP traffic, end-to-end TLS, client certificates (mTLS), or HTTP/2 passthrough | Default `tcp` (omit `mode`) |
| A fixed HTTPS/WSS service reachable from the daemon, with remote clients using loopback HTTP/WS and the daemon supplying upstream Host/SNI | `http-gateway` |

HTTPS plus WSS does not require gateway mode: both work over default TCP forwarding.
An `https://` endpoint alone does not enable gateway mode. In TCP mode, connecting
to `https://127.0.0.1:<port>` may fail certificate validation; the client must use
the upstream hostname for TLS and Host.

For example, keep TLS end-to-end with this reverse tunnel:

```toml
[[channels]]
name      = "prefect-tcp"
hostname  = "edge-server"
direction = "remote->local"
remote    = "5443"
local     = "prefect.mmdl.zoedev.top:443"
# mode = "tcp" is the default
```

On the SSH server, this command connects through the tunnel while keeping the
upstream hostname for TLS verification and Host:

```sh
curl --connect-to prefect.mmdl.zoedev.top:443:127.0.0.1:5443 https://prefect.mmdl.zoedev.top/
```

For gateway mode, use the same configuration with `mode = "http-gateway"`.
Then use `curl http://127.0.0.1:5443/` or a WebSocket client pointed at
`ws://127.0.0.1:5443/<actual-websocket-path>`. Both go to the same TLS upstream;
`local = "https://prefect.mmdl.zoedev.top/"` is equivalent to the host:port form.
Clients must support HTTP/WS loopback URLs. The daemon verifies upstream
certificates using native roots; this mode provides HTTP/1.1 and WebSocket
forwarding, without upstream client-certificate configuration or response
redirect/cookie/HTML URL rewriting. Applications that generate absolute URLs
may need their public/base URL configured to match how clients access them.

### Web status page

The Web status page is enabled by default, and its loopback URL is printed at startup. It shows channel endpoints, health, retries, latest errors, and host-key remediation commands.

**Open local** opens the channel's local endpoint. Set `[web].enabled = false` to disable the page; see the [configuration reference](docs/configuration.md) for other options.

### Credentials

`~/.ssh/config` can't hold passwords or key passphrases. If SSH config alone can't authenticate to a host, add an `[auth.<alias>]` block keyed by its alias:

```toml
[auth.my-db]
password   = "..."          # for password-auth hosts (no IdentityFile in SSH config)
# or
passphrase = "..."          # for encrypted IdentityFile
```

`password` overrides any `IdentityFile`. Hosts that authenticate cleanly via SSH config alone don't need an `[auth.*]` block at all.

### Reconnection (global)

```toml
[reconnection]
# max_retries             = 0     # default: 0 = unlimited
# initial_delay_secs      = 1     # default: 1 second
# max_delay_secs          = 30    # default: 30 seconds
# use_exponential_backoff = true  # default: true
```

Omit this section to use all defaults.

Each retry delay includes jitter. After a finite retry cycle is exhausted, automatic recovery continues with a second exponential backoff capped at 60 seconds; a successful session resets both counters. SSH handshakes are serialized to avoid reconnect storms.

### More examples

#### Share the tunnel on the local network

**Scenario:** Let other LAN machines use a database tunnel on this machine.

**Configuration:**

```toml
[[channels]]
name      = "shared-db"
hostname  = "db-server"
direction = "local->remote"
local     = "0.0.0.0:3306"
remote    = "3306"
```

**Explanation:** The service listens on `0.0.0.0:3306` and forwards connections to `127.0.0.1:3306` on `db-server`. Restrict access with a firewall.

#### Expose a local-network service to the SSH server

**Scenario:** Expose a service on the local network to `edge-server` with `remote->local` (`ssh -R`).

**Configuration:**

```toml
[[channels]]
name      = "lan-api"
hostname  = "edge-server"
direction = "remote->local"
local     = "https://api.internal.example/v1"
remote    = "18443" # edge-server binds 127.0.0.1:18443
```

**Explanation:** For every connection to `127.0.0.1:18443` on `edge-server`, this machine resolves `api.internal.example` and connects to port 443. Clients must include `/v1` in their request URLs; the configured path is not added automatically. To accept external connections on the server, set `remote = "0.0.0.0:18443"` and configure `GatewayPorts clientspecified` in `sshd_config`.

Full field reference: [docs/configuration.md](docs/configuration.md).

## Commands


| Command                   | What it does                                                                                                                                                                         |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `start`                   | Run in the foreground (Ctrl+C to stop).                                                                                                                                              |
| `start -D` / `--daemon`   | Spawn a detached background process.                                                                                                                                                 |
| `stop`                    | Tell the running process to exit gracefully (via IPC).                                                                                                                               |
| `restart`                 | Stop the running service, then restart it as a daemon.                                                                                                                               |
| `status`                  | Show service state, per-channel health (Connected / Reconnecting / Failed / Stopped), PID, and endpoints. Add `--watch / -w` to refresh every `--interval / -n` seconds (default 2). |
| `test`                    | Probe each configured `local->remote` listener to confirm the tunnel is alive. `remote->local` channels are skipped — verify those server-side.                                      |
| `validate`                | Resolve every channel against `~/.ssh/config` and report any problems.                                                                                                               |
| `generate -o config.toml` | Scaffold a `config.toml` from existing SSH config aliases.                                                                                                                           |
| `hosts`                   | Scan SSH config aliases and show whether each host is supported. Use `--format json` for script-friendly output.                                                                     |


All commands accept `--config /path/to/config.toml` to point at a non-default file, and `--debug` for verbose logging.

## Further reading

- [Troubleshooting](docs/troubleshooting.md) — common connection, host-key, and port issues.
- [Configuration reference](docs/configuration.md) — every field, every edge case.
- [How to use](docs/HowToUse.md) — task-oriented walkthroughs.
- [Connection testing](docs/testing.md) — verify configured channels.
- [Architecture](docs/architecture.md) — how channels, sessions, and reconnection fit together.

## License

MIT — see [LICENSE](LICENSE).
