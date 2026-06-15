/// Error type for setdns operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

/// Result type for setdns operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;
