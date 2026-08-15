use crate::config::{Direction, WebConfig};
use crate::error::{AppError, Result};
use crate::service::{ChannelHealth, ServiceManager, ServiceState, ServiceStatus};
use std::fmt::Write as _;
use std::io::ErrorKind;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

const WEB_HOST: &str = "127.0.0.1";

/// Start the loopback Web status server and return its actual bound port.
pub async fn start(
  config: &WebConfig,
  service_manager: Arc<ServiceManager>,
  cancel: CancellationToken,
) -> Result<u16> {
  let (listener, port) = bind(config).await?;

  tokio::spawn(async move {
    loop {
      tokio::select! {
        _ = cancel.cancelled() => break,
        accepted = listener.accept() => match accepted {
          Ok((stream, _)) => {
            let manager = Arc::clone(&service_manager);
            tokio::spawn(async move {
              if let Err(error) = handle_connection(stream, manager).await {
                debug!(error = ?error, "Web status connection failed");
              }
            });
          }
          Err(error) => {
            warn!(error = ?error, "Web status listener stopped");
            break;
          }
        }
      }
    }
  });

  info!(port, "Web status page started");
  Ok(port)
}

async fn bind(config: &WebConfig) -> Result<(TcpListener, u16)> {
  for port in config.port..=u16::MAX {
    match TcpListener::bind((WEB_HOST, port)).await {
      Ok(listener) => {
        let actual_port = listener.local_addr().map_err(|error| {
          AppError::Service(format!("Failed to inspect Web status listener: {}", error))
        })?;
        return Ok((listener, actual_port.port()));
      }
      Err(error) if !config.strict && error.kind() == ErrorKind::AddrInUse => {
        debug!(port, "Web status port occupied, trying the next port");
      }
      Err(error) => {
        return Err(AppError::Service(format!(
          "Failed to bind Web status page on http://{}:{}: {}",
          WEB_HOST, port, error
        )));
      }
    }
  }

  Err(AppError::Service(format!(
    "No available Web status port at or above {}",
    config.port
  )))
}

async fn handle_connection(
  mut stream: TcpStream,
  service_manager: Arc<ServiceManager>,
) -> std::io::Result<()> {
  let request_line = {
    let mut request_line = String::new();
    BufReader::new(&mut stream)
      .take(8192)
      .read_line(&mut request_line)
      .await?;
    request_line
  };

  let is_head = request_line.starts_with("HEAD / ");
  if !is_head && !request_line.starts_with("GET / ") {
    stream
      .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
      .await?;
    return stream.shutdown().await;
  }

  let body = render(&service_manager.status().await);
  let headers = format!(
    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
    body.len()
  );
  stream.write_all(headers.as_bytes()).await?;
  if !is_head {
    stream.write_all(body.as_bytes()).await?;
  }
  stream.shutdown().await
}

fn escape_html(value: &str) -> String {
  value
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&#39;")
}

fn local_url(endpoint: &str) -> Option<String> {
  let (host, port) = endpoint.rsplit_once(':')?;
  let host = host
    .strip_prefix('[')
    .and_then(|value| value.strip_suffix(']'))
    .unwrap_or(host);
  let host = match host {
    "0.0.0.0" | "::" => WEB_HOST,
    other => other,
  };
  let host = if host.contains(':') {
    format!("[{}]", host)
  } else {
    host.to_string()
  };
  Some(format!("http://{}:{}", host, port))
}

fn health_view(health: &ChannelHealth) -> (&'static str, &'static str, Option<String>) {
  match health {
    ChannelHealth::Stopped => ("Stopped", "stopped", None),
    ChannelHealth::Connecting { attempt } => (
      "Connecting",
      "pending",
      (*attempt > 1).then(|| format!("Attempt {}", attempt)),
    ),
    ChannelHealth::Connected => ("Connected", "connected", None),
    ChannelHealth::Reconnecting {
      attempt,
      last_error,
    } => (
      "Reconnecting",
      "pending",
      Some(format!("Attempt {}: {}", attempt, last_error)),
    ),
    ChannelHealth::Failed { error } => ("Failed", "failed", Some(error.clone())),
  }
}

fn render(status: &ServiceStatus) -> String {
  let state = match status.state {
    ServiceState::Stopped => "Stopped",
    ServiceState::Starting => "Starting",
    ServiceState::Running => "Running",
    ServiceState::Stopping => "Stopping",
    ServiceState::Error(_) => "Error",
  };
  let state_class = if matches!(status.state, ServiceState::Running) {
    "connected"
  } else if matches!(status.state, ServiceState::Error(_)) {
    "failed"
  } else {
    "pending"
  };

  let mut rows = String::new();
  for channel in &status.channels {
    let (health, health_class, detail) = health_view(&channel.health);
    let (direction_class, direction_label, direction_arrow) = match channel.direction {
      Direction::LocalToRemote => ("outbound", "SSH -L", "->"),
      Direction::RemoteToLocal => ("inbound", "SSH -R", "<-"),
    };
    let action = local_url(&channel.local)
      .map(|url| {
        format!(
          "<a class=\"open\" href=\"{}\" target=\"_blank\" rel=\"noreferrer\">Open local</a>",
          escape_html(&url)
        )
      })
      .unwrap_or_default();
    let detail = detail
      .map(|value| {
        format!(
          "<small class=\"health-detail\">{}</small>",
          escape_html(&value)
        )
      })
      .unwrap_or_default();

    let _ = write!(
      rows,
      "<tr><td class=\"channel\" data-label=\"Channel\"><strong>{}</strong><span>{}</span></td><td class=\"route-cell\" data-label=\"Route\"><div class=\"route\"><div class=\"endpoint local\"><span>Local</span><code>{}</code></div><div class=\"rail {}\" aria-label=\"{}\"><i></i><b>{}</b></div><div class=\"endpoint remote\"><span>Remote</span><code>{}</code></div></div></td><td class=\"health-cell\" data-label=\"Health\"><span class=\"health {}\"><i></i>{}</span>{}</td><td class=\"action\">{}</td></tr>",
      escape_html(&channel.name),
      direction_label,
      escape_html(&channel.local),
      direction_class,
      channel.direction.as_arrow(),
      direction_arrow,
      escape_html(&channel.remote),
      health_class,
      health,
      detail,
      action,
    );
  }

  if rows.is_empty() {
    rows.push_str("<tr><td class=\"empty\" colspan=\"4\"><div class=\"empty-route\"><i></i><span></span><i></i></div><strong>No channels configured</strong></td></tr>");
  }

  let connected = status.connected_count();
  let total = status.total_count();
  let coverage = (connected * 100).checked_div(total).unwrap_or(0);

  format!(
    r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="refresh" content="3">
<title>SSH Channels Hub</title>
<style>
:root{{--canvas:#f2f4f3;--surface:#fbfcfb;--surface-hover:#f7f9f8;--ink:#17201d;--secondary:#53605b;--tertiary:#75807c;--muted:#919a96;--line:rgba(23,32,29,.12);--line-soft:rgba(23,32,29,.07);--line-strong:rgba(23,32,29,.2);--green:#14734a;--green-soft:#e4f3eb;--amber:#8a5b08;--amber-soft:#fff2d5;--red:#a33b32;--red-soft:#fbe9e7;--blue:#215f8d;--blue-soft:#e8f1f7}}
*{{box-sizing:border-box}}
body{{min-width:320px;min-height:100vh;margin:0;background:var(--canvas);color:var(--ink);font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif;letter-spacing:0}}
.frame{{width:min(1180px,calc(100% - 40px));margin:0 auto;padding:36px 0 64px}}
.topbar{{display:flex;align-items:center;justify-content:space-between;gap:32px;padding-bottom:28px;border-bottom:1px solid var(--line)}}
.brand{{display:flex;align-items:center;gap:14px}}
.mark{{position:relative;width:38px;height:38px;flex:0 0 38px;border:1px solid var(--line-strong);border-radius:6px;background:var(--surface)}}
.mark::before,.mark::after{{content:"";position:absolute;left:8px;width:5px;height:5px;border:2px solid var(--green);border-radius:50%;background:var(--surface)}}
.mark::before{{top:8px}}.mark::after{{bottom:8px}}
.mark i{{position:absolute;left:11px;top:14px;width:14px;height:8px;border-right:2px solid var(--blue);border-bottom:2px solid var(--blue)}}
.kicker,.section-heading p{{margin:0;color:var(--tertiary);font-size:10px;font-weight:750;text-transform:uppercase}}
h1{{margin:1px 0 0;font-size:21px;line-height:1.25;font-weight:720;letter-spacing:0}}
.overview{{display:flex;align-items:center;gap:22px}}
.service,.coverage{{display:grid;gap:2px}}.coverage{{position:relative;padding-bottom:7px}}
.service-label,.coverage small{{color:var(--tertiary);font-size:11px}}
.divider{{width:1px;height:34px;background:var(--line)}}
.coverage span{{font-variant-numeric:tabular-nums;line-height:1}}
.coverage strong{{font-size:18px}}.coverage em{{color:var(--tertiary);font-style:normal;font-weight:650}}
.health{{display:inline-flex;align-items:center;gap:7px;width:max-content;font-weight:720}}
.health i{{width:7px;height:7px;border-radius:50%;background:currentColor;box-shadow:0 0 0 3px color-mix(in srgb,currentColor 12%,transparent)}}
.connected{{color:var(--green)}}.pending{{color:var(--amber)}}.failed{{color:var(--red)}}.stopped{{color:var(--muted)}}
.coverage-bar{{position:absolute;left:0;bottom:0;height:2px;max-width:100%;border-radius:1px;background:var(--green)}}
.section-heading{{display:flex;align-items:flex-end;justify-content:space-between;gap:20px;margin:30px 0 12px}}
.section-heading h2{{margin:2px 0 0;font-size:16px;line-height:1.3;letter-spacing:0}}
.count{{color:var(--tertiary);font-size:12px;font-variant-numeric:tabular-nums}}
.panel{{overflow:hidden;background:var(--surface);border:1px solid var(--line);border-radius:6px}}
table{{width:100%;border-collapse:collapse}}
th,td{{padding:18px;text-align:left;border-bottom:1px solid var(--line-soft);vertical-align:middle}}
th{{padding-top:11px;padding-bottom:11px;color:var(--muted);font-size:10px;font-weight:750;text-transform:uppercase}}
tbody tr{{transition:background-color 140ms ease}}tbody tr:hover{{background:var(--surface-hover)}}tr:last-child td{{border-bottom:0}}
.channel{{width:180px}}.channel strong{{display:block;font-size:14px;font-weight:720;overflow-wrap:anywhere}}.channel span{{display:block;margin-top:2px;color:var(--tertiary);font:10px/1.4 ui-monospace,SFMono-Regular,Consolas,monospace}}
.route-cell{{width:52%}}
.route{{display:grid;grid-template-columns:minmax(110px,1fr) 92px minmax(110px,1fr);align-items:center;gap:10px}}
.endpoint{{min-width:0}}.endpoint span{{display:block;margin-bottom:3px;color:var(--muted);font-size:9px;font-weight:750;text-transform:uppercase}}.endpoint code{{display:block;color:var(--secondary);font:12px/1.4 ui-monospace,SFMono-Regular,Consolas,monospace;overflow-wrap:anywhere}}
.rail{{position:relative;display:flex;align-items:center;justify-content:center;height:28px;color:var(--blue)}}
.rail i{{position:absolute;left:0;right:0;top:13px;height:1px;background:var(--line-strong)}}
.rail i::before,.rail i::after{{content:"";position:absolute;top:-3px;width:7px;height:7px;border:2px solid var(--blue);border-radius:50%;background:var(--surface)}}
.rail i::before{{left:0}}.rail i::after{{right:0}}
.rail b{{position:relative;z-index:1;padding:1px 7px;background:var(--surface);font:700 11px/1.5 ui-monospace,SFMono-Regular,Consolas,monospace}}
tr:hover .rail b,tr:hover .rail i::before,tr:hover .rail i::after{{background:var(--surface-hover)}}
.health-cell{{width:170px}}.health-detail{{display:block;max-width:260px;margin-top:4px;color:var(--tertiary);font-size:11px;overflow-wrap:anywhere}}
.action{{width:112px;text-align:right}}
.open{{display:inline-flex;align-items:center;min-height:32px;padding:6px 10px;border:1px solid var(--line-strong);border-radius:4px;color:var(--blue);background:var(--surface);font-size:12px;font-weight:720;text-decoration:none;white-space:nowrap;transition:color 140ms ease,background-color 140ms ease,border-color 140ms ease}}
.open:hover{{border-color:var(--blue);color:var(--surface);background:var(--blue)}}.open:focus-visible{{outline:2px solid var(--blue);outline-offset:2px}}
.empty{{padding:72px 24px;text-align:center;color:var(--tertiary)}}.empty strong{{display:block;margin-top:14px;color:var(--secondary);font-size:13px}}
.empty-route{{display:flex;align-items:center;justify-content:center;width:132px;margin:0 auto}}.empty-route i{{width:9px;height:9px;border:2px solid var(--muted);border-radius:50%}}.empty-route span{{width:96px;height:1px;background:var(--line-strong)}}
@media(prefers-color-scheme:dark){{:root{{--canvas:#111614;--surface:#171d1a;--surface-hover:#1b221f;--ink:#e7ece9;--secondary:#b4bfba;--tertiary:#84918b;--muted:#66736d;--line:rgba(231,236,233,.13);--line-soft:rgba(231,236,233,.08);--line-strong:rgba(231,236,233,.22);--green:#58b98a;--green-soft:#173b2a;--amber:#d8a847;--amber-soft:#3d3016;--red:#df7b71;--red-soft:#40201e;--blue:#70acd4;--blue-soft:#193247}}}}
@media(max-width:760px){{.frame{{width:calc(100% - 24px);padding:24px 0 40px}}.topbar{{align-items:flex-start;flex-direction:column;gap:20px;padding-bottom:20px}}.overview{{width:100%;max-width:100%;justify-content:space-between}}.divider{{margin-left:auto}}.section-heading{{margin-top:24px}}table,tbody{{display:block;width:100%}}thead{{display:none}}tbody tr{{display:grid;width:100%;min-width:0;grid-template-columns:minmax(0,1fr) auto;padding:16px 14px;border-bottom:1px solid var(--line-soft)}}td{{display:block;min-width:0;padding:4px 0;border:0}}td::before{{content:attr(data-label);display:block;margin-bottom:3px;color:var(--muted);font-size:9px;font-weight:750;text-transform:uppercase}}.channel{{width:auto}}.route-cell,.health-cell{{grid-column:1/-1;width:auto;margin-top:12px}}.route{{grid-template-columns:minmax(0,1fr) 54px minmax(0,1fr);gap:6px}}.rail b{{padding:1px 4px}}.action{{grid-column:2;grid-row:1;width:auto;padding-left:10px;align-self:start}}.health-detail{{max-width:none}}}}
</style>
</head>
<body><main class="frame"><header class="topbar"><div class="brand"><span class="mark" aria-hidden="true"><i></i></span><div><p class="kicker">SSH Channels Hub</p><h1>Channel routes</h1></div></div><div class="overview"><div class="service"><span class="health {state_class}"><i></i>{state}</span><span class="service-label">Service</span></div><span class="divider" aria-hidden="true"></span><div class="coverage"><span><strong>{connected}</strong><em> / {total}</em></span><small>Connected</small><span class="coverage-bar" style="width:{coverage}%"></span></div></div></header><div class="section-heading"><div><p>Route ledger</p><h2>Forwarded channels</h2></div><span class="count">{total} total</span></div><section class="panel" aria-label="Channel status"><table><thead><tr><th>Channel</th><th>Local / remote route</th><th>Health</th><th></th></tr></thead><tbody>{rows}</tbody></table></section></main></body>
</html>"#,
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::service::{ChannelStatus, ServiceStatus};

  #[test]
  fn renders_escaped_channels_and_links_both_directions_to_local() {
    let status = ServiceStatus {
      state: ServiceState::Running,
      channels: vec![
        ChannelStatus {
          name: "web <prod>".into(),
          direction: Direction::LocalToRemote,
          local: "0.0.0.0:8080".into(),
          remote: "web.internal:80".into(),
          health: ChannelHealth::Connected,
        },
        ChannelStatus {
          name: "reverse".into(),
          direction: Direction::RemoteToLocal,
          local: "127.0.0.1:3000".into(),
          remote: "127.0.0.1:9000".into(),
          health: ChannelHealth::Failed {
            error: "bad <key>".into(),
          },
        },
      ],
    };

    let page = render(&status);
    assert!(page.contains("web &lt;prod&gt;"));
    assert!(page.contains("href=\"http://127.0.0.1:8080\""));
    assert!(page.contains("href=\"http://127.0.0.1:3000\""));
    assert!(!page.contains("href=\"http://127.0.0.1:9000\""));
    assert_eq!(page.matches("class=\"open\"").count(), 2);
    assert!(page.contains("class=\"rail inbound\""));
    assert!(page.contains("bad &lt;key&gt;"));
  }

  #[test]
  fn local_url_formats_ipv6() {
    assert_eq!(local_url("::1:3306").as_deref(), Some("http://[::1]:3306"));
    assert_eq!(
      local_url("[::1]:3306").as_deref(),
      Some("http://[::1]:3306")
    );
  }

  #[tokio::test]
  async fn occupied_port_falls_forward_unless_strict() {
    let (_, ephemeral_port) = bind(&WebConfig {
      port: 0,
      strict: true,
      ..WebConfig::default()
    })
    .await
    .unwrap();
    assert_ne!(ephemeral_port, 0);

    let occupied = TcpListener::bind((WEB_HOST, 0)).await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    if port == u16::MAX {
      return;
    }

    let flexible = WebConfig {
      port,
      ..WebConfig::default()
    };
    let (_, actual_port) = bind(&flexible).await.unwrap();
    assert!(actual_port > port);

    let strict = WebConfig {
      strict: true,
      ..flexible
    };
    assert!(bind(&strict).await.is_err());
  }
}
