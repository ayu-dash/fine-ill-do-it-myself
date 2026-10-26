use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse manifest: {0}")]
    Manifest(#[from] toml::de::Error),
    #[error("binary `{0}` not found in manifest")]
    MissingBinary(String),
    #[error("unsupported platform")]
    UnsupportedPlatform,
    #[error("checksum mismatch for {0}")]
    ChecksumMismatch(String),
    #[error("php version {0} not installed")]
    PhpNotInstalled(String),
    #[error("service `{0}` is not running")]
    NotRunning(String),
    #[error("port {0} is already in use; try {1}")]
    PortConflict(u16, String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
