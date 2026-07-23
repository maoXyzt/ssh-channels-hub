use thiserror::Error;

/// Application error types
#[derive(Error, Debug)]
pub enum AppError {
  #[error("Configuration error: {0}")]
  Config(String),

  #[error("SSH connection error: {0}")]
  SshConnection(String),

  #[error("SSH authentication error: {0}")]
  SshAuthentication(String),

  #[error("SSH host key error: {0}")]
  SshHostKey(String),

  #[error("SSH channel error: {0}")]
  SshChannel(String),

  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("Configuration parse error: {0}")]
  ConfigParse(#[from] toml::de::Error),

  #[error("Service error: {0}")]
  Service(String),
}

impl AppError {
  /// Only transport/session failures can recover without changing local
  /// configuration or credentials.
  pub fn is_retryable(&self) -> bool {
    matches!(self, Self::SshConnection(_))
  }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_connection_errors_are_retryable() {
    assert!(AppError::SshConnection("reset".into()).is_retryable());
    assert!(!AppError::SshAuthentication("rejected".into()).is_retryable());
    assert!(!AppError::SshHostKey("unknown".into()).is_retryable());
    assert!(!AppError::SshChannel("bind failed".into()).is_retryable());
  }
}
