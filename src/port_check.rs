use crate::error::{AppError, Result};
use std::net::TcpListener;
use std::time::Duration;
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::timeout;

/// Check if a `host:port` is available to bind. Tries to bind there; success means available.
pub async fn is_port_available(host: &str, port: u16) -> Result<bool> {
  let addr = format!("{}:{}", host, port).parse().map_err(|e| {
    AppError::Io(std::io::Error::new(
      std::io::ErrorKind::InvalidInput,
      format!("Invalid address {}:{}: {}", host, port, e),
    ))
  })?;
  let socket = match addr {
    std::net::SocketAddr::V4(_) => TcpSocket::new_v4(),
    std::net::SocketAddr::V6(_) => TcpSocket::new_v6(),
  }
  .map_err(|e| {
    AppError::Io(std::io::Error::other(format!(
      "Failed to create socket: {}",
      e
    )))
  })?;

  match socket.bind(addr) {
    Ok(_) => Ok(true),
    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Ok(false),
    Err(e) => Err(AppError::Io(e)),
  }
}

/// Synchronous variant of [`is_port_available`], for blocking contexts.
#[allow(dead_code)]
pub fn is_port_available_sync(host: &str, port: u16) -> Result<bool> {
  match TcpListener::bind(format!("{}:{}", host, port)) {
    Ok(_) => Ok(true),
    Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => Ok(false),
    Err(e) => Err(AppError::Io(e)),
  }
}

/// Check multiple `host:port` pairs and return the list of those already in use.
pub async fn check_ports(endpoints: &[(String, u16)]) -> Result<Vec<(String, u16)>> {
  let mut occupied = Vec::new();
  for (host, port) in endpoints {
    if !is_port_available(host, *port).await? {
      occupied.push((host.clone(), *port));
    }
  }
  Ok(occupied)
}

/// Test if a TCP connection can be established to a port
/// This is useful for verifying that a port forwarding channel is actually working
pub async fn test_port_connection(host: &str, port: u16) -> Result<bool> {
  let addr = format!("{}:{}", host, port);

  // Try to connect with a timeout
  match timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await {
    Ok(Ok(_)) => Ok(true),
    Ok(Err(_)) => Ok(false),
    Err(_) => Ok(false), // Timeout
  }
}

/// Test if an SSH tunnel is actually working by attempting to send/receive data
/// This detects cases where the local port is listening but the SSH connection is dead
pub async fn test_tunnel_connection(host: &str, port: u16) -> Result<bool> {
  use tokio::io::AsyncWriteExt;

  let addr = format!("{}:{}", host, port);

  // Try to connect with a timeout
  let mut stream = match timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await {
    Ok(Ok(s)) => s,
    Ok(Err(_)) => return Ok(false),
    Err(_) => return Ok(false), // Timeout
  };

  // Try to send a small amount of data to verify the tunnel is working
  // If the SSH connection is dead, this will fail with connection reset
  match timeout(Duration::from_secs(1), stream.write_all(b"X")).await {
    Ok(Ok(_)) => {
      // Successfully sent data, tunnel appears to be working
      Ok(true)
    }
    Ok(Err(e)) => {
      // Check if it's a connection reset error (SSH tunnel is dead)
      if e.kind() == std::io::ErrorKind::ConnectionReset
        || e.kind() == std::io::ErrorKind::BrokenPipe
      {
        Ok(false)
      } else {
        // Other error, but connection was established, so consider it working
        Ok(true)
      }
    }
    Err(_) => {
      // Timeout on write, but connection was established
      // This might happen if the remote service doesn't respond, but tunnel is working
      Ok(true)
    }
  }
}

// /// Test multiple port connections and return results
// pub async fn test_port_connections(connections: &[(String, u16)]) -> Vec<(String, u16, bool)> {
//     let mut results = Vec::new();

//     for (host, port) in connections {
//         let connected = test_port_connection(host, *port).await.unwrap_or(false);
//         results.push((host.clone(), *port, connected));
//     }

//     results
// }

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn test_port_check() {
    // Test with a random high port (likely to be available)
    let port = 49152
      + (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        % 16384) as u16;

    let available = is_port_available("127.0.0.1", port).await;
    assert!(available.is_ok());
  }

  #[tokio::test]
  async fn occupied_port_is_detected_on_specific_host() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    assert!(!is_port_available("127.0.0.1", port).await.unwrap());
  }
}
