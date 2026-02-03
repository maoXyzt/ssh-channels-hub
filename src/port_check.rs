use crate::error::{AppError, Result};
use std::net::TcpListener;
use std::time::Duration;
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::timeout;

/// Check if a port is available (not in use)
///
/// This function attempts to bind to the specified port on localhost.
/// If the bind succeeds, the port is available. If it fails, the port is likely in use.
pub async fn is_port_available(port: u16) -> Result<bool> {
    // Try to bind to the port using tokio::net::TcpSocket
    // This is the async way and works on all platforms
    let socket = TcpSocket::new_v4().map_err(|e| {
        AppError::Io(std::io::Error::other(format!(
            "Failed to create socket: {}",
            e
        )))
    })?;

    // Try to bind to the port
    match socket.bind(format!("127.0.0.1:{}", port).parse().map_err(|e| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Invalid address: {}", e),
        ))
    })?) {
        Ok(_) => {
            // Port is available
            Ok(true)
        }
        Err(e) => {
            // Check if the error is because the port is already in use
            if e.kind() == std::io::ErrorKind::AddrInUse {
                Ok(false)
            } else {
                // Some other error occurred
                Err(AppError::Io(e))
            }
        }
    }
}

/// Check if a port is available synchronously (for blocking contexts)
///
/// This is a fallback method that uses std::net::TcpListener
#[allow(dead_code)]
pub fn is_port_available_sync(port: u16) -> Result<bool> {
    match TcpListener::bind(format!("127.0.0.1:{}", port)) {
        Ok(_) => Ok(true),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                Ok(false)
            } else {
                Err(AppError::Io(e))
            }
        }
    }
}

/// Check multiple ports and return a list of occupied ports
pub async fn check_ports(ports: &[u16]) -> Result<Vec<u16>> {
    let mut occupied = Vec::new();

    for &port in ports {
        if !is_port_available(port).await? {
            occupied.push(port);
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

        let available = is_port_available(port).await;
        assert!(available.is_ok());
    }
}
