# SSH Channels Hub

> English | [中文](./README-zh.md)

Declarative SSH tunnels with auto-reconnect. Define your port forwards once in TOML, start one service, and they all come up — reconnecting automatically when the link drops.

Cross-platform (Linux, macOS, Windows). Written in Rust on top of [russh](https://docs.rs/russh).

## Why

Reach for this when `ssh -L 3306:127.0.0.1:3306 db.example.com` has grown into *"I have five of those, my laptop sleeps, my Wi-Fi flakes, and I want them all back when I open the lid."*

- **Declarative**: tunnels live in `config.toml`, not in shell history or terminal panes.
- **No host config duplication**: host info (`HostName` / `User` / `Port` / `IdentityFile`) is read straight from `~/.ssh/config` — you reference aliases.
- **ProxyJump aware**: chain through bastions defined in `~/.ssh/config` — alias-only references, publickey auth, and strict `known_hosts` checks for targets and jumps. See [docs/configuration.md §3.4](docs/configuration.md#34-host-info-从哪里来).
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

The wheel installs the same `ssh-channels-hub` binary on Linux x86_64,
macOS arm64, and Windows x86_64; it does not run through Python.

If you already have a Cargo toolchain, you can also install with:

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

**2. Have the host in `~/.ssh/config`**

```
Host my-db
  HostName db.example.com
  User myuser
  IdentityFile ~/.ssh/id_rsa
```

**3. Write `config.toml`** in the current directory:

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
# or, after pip/cargo installation:
ssh-channels-hub start          # Ctrl+C to stop
# or, after cargo build:
./target/release/ssh-channels-hub start       # Linux/macOS
.\target\release\ssh-channels-hub.exe start  # Windows PowerShell
```

Now `mysql -h 127.0.0.1 -P 3306` goes through the tunnel.

> **Tip:** `ssh-channels-hub generate -o config.toml` scaffolds one commented-out `[[channels]]` block per alias in your SSH config — uncomment and fill in ports. Or `cp config.example.toml config.toml` for an annotated template.

## Configuration

`config.toml` is looked up in this order (first existing wins):

| Platform | Path |
|---|---|
| Current directory (always tried first) | `./config.toml` |
| Linux / macOS | `~/.config/ssh-channels-hub/config.toml` |
| Windows | `%APPDATA%\ssh-channels-hub\config.toml` |

`--config /path/to/file` overrides the lookup.

### Channel schema

```toml
[[channels]]
name      = "string"                            # required, unique identifier
hostname  = "ssh-config-alias"                  # required; resolves via ~/.ssh/config
direction = "local->remote" | "remote->local"   # required
local     = "port" | "host:port"                # required, this machine's side
remote    = "port" | "host:port"                # required, the SSH server's side
```

`local` and `remote` always name the address on their respective side regardless of direction. Direction decides who listens:

- **`local->remote`** (≈ `ssh -L`): this machine listens on `local`; the server dials `remote` for each connection.
- **`remote->local`** (≈ `ssh -R`): the server binds `remote`; incoming traffic is bridged to `local` on this side.

Endpoints accept:
- `"3306"` → `127.0.0.1:3306` (bare port, host defaults to loopback)
- `"127.0.0.1:3306"` → explicit form
- `"0.0.0.0:8080"` → bind on every interface
- `"[::1]:3306"` → IPv6

### Web status page

Starting the service also serves a live channel dashboard on loopback. It shows
the service summary, each channel's direction, local and remote endpoints,
health, retry attempt, and latest error. The actual URL is printed for both
foreground and daemon startup.

Every channel has an **Open local** link built from its `local` endpoint. This
also applies to `remote->local` channels: the link opens the local service being
exposed, never the remote bind address.

```toml
[web]
enabled = true   # default: true; set false to disable
port = 9090      # default: 9090; preferred port
strict = false   # default: false; if occupied, try 9091, 9092, ...
```

### Credentials

`~/.ssh/config` can't hold passwords or key passphrases. When SSH config alone can't authenticate the host, add an `[auth.<alias>]` block keyed by the SSH config alias:

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
max_retries             = 0     # 0 = unlimited
initial_delay_secs      = 1
max_delay_secs          = 30
use_exponential_backoff = true
```

Each retry delay includes jitter. After a finite retry cycle is exhausted,
automatic recovery continues with a second exponential backoff capped at 60
seconds; a successful session resets both counters. SSH handshakes are
serialized to avoid reconnect storms.

### More examples

#### Share the tunnel on the local network

Listen on every interface so other LAN machines can use the tunnel (mind your
firewall):

```toml
[[channels]]
name      = "shared-db"
hostname  = "db-server"
direction = "local->remote"
local     = "0.0.0.0:3306"
remote    = "3306"
```

#### Expose a local-network service to the SSH server

With `remote->local` (`ssh -R`), `local` can point to another service reachable
from this machine instead of loopback:

```toml
[[channels]]
name      = "lan-api"
hostname  = "edge-server"
direction = "remote->local"
local     = "192.168.1.50:3000" # bare "3000" means 127.0.0.1:3000
remote    = "8080"              # edge-server binds 127.0.0.1:8080
```

This exposes `192.168.1.50:3000` at `127.0.0.1:8080` on `edge-server`.

(For the server to bind `0.0.0.0:8080`, set `remote = "0.0.0.0:8080"` **and** set `GatewayPorts clientspecified` in the server's `sshd_config`.)

Full field reference: [docs/configuration.md](docs/configuration.md).

## Commands

| Command | What it does |
|---|---|
| `start` | Run in the foreground (Ctrl+C to stop). |
| `start -D` / `--daemon` | Spawn a detached background process. |
| `stop` | Tell the running process to exit gracefully (via IPC). |
| `restart` | Stop the running service, then re-start as daemon. |
| `status` | Show service state, per-channel health (Connected / Reconnecting / Failed / Stopped), PID, and endpoints. Add `--watch / -w` to refresh every `--interval / -n` seconds (default 2). |
| `test` | Probe each configured `local->remote` listener to confirm the tunnel is alive. `remote->local` channels are skipped — verify those server-side. |
| `validate` | Resolve every channel against `~/.ssh/config` and report any problems. |
| `generate -o config.toml` | Scaffold a `config.toml` from existing SSH config aliases. |
| `hosts` | Scan SSH config aliases and show whether each host is supported. Use `--format json` for script-friendly output. |

All commands accept `--config /path/to/config.toml` to point at a non-default file, and `--debug` for verbose logging.

## Troubleshooting

- **`Channel '...' references host alias '...', but no Host ... block exists`** — typo in `hostname`, or the alias is missing from `~/.ssh/config`.
- **`Address(es) already in use`** — something else is bound to your `local` address. Change the port or stop the other process. Find the culprit with `lsof -i :PORT` (Linux/macOS) or `netstat -ano | findstr :PORT` (Windows).
- **Bind ports < 1024** — needs root (Linux/macOS) or Administrator (Windows).
- **Connection fails** — `ssh <alias>` manually first to isolate SSH config / network / key permission issues.
- **Encrypted key not unlocking** — set `[auth.<alias>] passphrase = "..."`.
- **Full debug output** — `ssh-channels-hub start --debug` logs each channel's SSH handshake, channel open, and reconnection attempts.

## Further reading

- [Configuration reference](docs/configuration.md) — every field, every edge case.
- [How to use](docs/HowToUse.md) — task-oriented walkthroughs.
- [Architecture](docs/architecture.md) — how channels, sessions, and reconnection fit together.

## License

MIT — see [LICENSE](LICENSE).
