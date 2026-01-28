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

    #[error("SSH channel error: {0}")]
    SshChannel(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration parse error: {0}")]
    ConfigParse(#[from] toml::de::Error),

    #[error("Service error: {0}")]
    Service(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
