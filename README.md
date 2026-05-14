# SSH Channels Hub

> English | [中文](./README-zh.md)

Declarative SSH tunnels with auto-reconnect. Define your port forwards once in TOML, start one service, and they all come up — reconnecting automatically when the link drops.

Cross-platform (Linux, macOS, Windows). Written in Rust on top of [russh](https://docs.rs/russh).

## Why

Reach for this when `ssh -L 3306:127.0.0.1:3306 db.example.com` has grown into *"I have five of those, my laptop sleeps, my Wi-Fi flakes, and I want them all back when I open the lid."*

- **Declarative**: tunnels live in `configs.toml`, not in shell history or terminal panes.
- **No host config duplication**: host info (`HostName` / `User` / `Port` / `IdentityFile`) is read straight from `~/.ssh/config` — you reference aliases.
- **Auto-reconnect**: configurable backoff per tunnel; one drop doesn't take the others down.
- **Both directions in one schema**: local-to-remote (`ssh -L`) and remote-to-local (`ssh -R`).
- **Foreground or daemon**: `start` attaches to the terminal, `start -D` detaches; `stop` / `restart` / `status` talk to the running process via IPC.

## Quickstart

**1. Install**

```bash
cargo install --path .          # from a clone
# or
cargo build --release           # binary at target/release/ssh-channels-hub
```

**2. Have the host in `~/.ssh/config`**

```
Host my-db
  HostName db.example.com
  User myuser
  IdentityFile ~/.ssh/id_rsa
```

**3. Write `configs.toml`** in the current directory:

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
ssh-channels-hub start          # Ctrl+C to stop
```

Now `mysql -h 127.0.0.1 -P 3306` goes through the tunnel.

> **Tip:** `ssh-channels-hub generate -o configs.toml` scaffolds one commented-out `[[channels]]` block per alias in your SSH config — uncomment and fill in ports. Or `cp configs.example.toml configs.toml` for an annotated template.

## Configuration

`configs.toml` is looked up in this order (first existing wins):

| Platform | Path |
|---|---|
| Current directory (always tried first) | `./configs.toml` |
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

### More examples

**Listen on every interface** so other LAN machines can use the tunnel (mind your firewall):

```toml
[[channels]]
name      = "shared-db"
hostname  = "db-server"
direction = "local->remote"
local     = "0.0.0.0:3306"
remote    = "3306"
```

**Expose a local service to the SSH server** (`ssh -R`):

```toml
[[channels]]
name      = "expose-local-web"
hostname  = "jumpbox"
direction = "remote->local"
remote    = "8022"              # server binds 127.0.0.1:8022
local     = "80"                # incoming traffic bridges to 127.0.0.1:80 here
```

(For the server to bind `0.0.0.0:8022`, set `remote = "0.0.0.0:8022"` **and** enable `GatewayPorts` in the server's `sshd_config`.)

Full field reference: [docs/configuration.md](docs/configuration.md).

## Commands

| Command | What it does |
|---|---|
| `start` | Run in the foreground (Ctrl+C to stop). |
| `start -D` / `--daemon` | Spawn a detached background process. |
| `stop` | Tell the running process to exit gracefully (via IPC). |
| `restart` | Stop the running service, then re-start as daemon. |
| `status` | Show state, active vs total channels, PID, and the channel list. |
| `test` | Probe each configured `local->remote` listener to confirm the tunnel is alive. `remote->local` channels are skipped — verify those server-side. |
| `validate` | Resolve every channel against `~/.ssh/config` and report any problems. |
| `generate -o configs.toml` | Scaffold a `configs.toml` from existing SSH config aliases. |

All commands accept `--config /path/to/configs.toml` to point at a non-default file, and `--debug` for verbose logging.

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
