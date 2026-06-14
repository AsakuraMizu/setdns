/// Error type for setdns operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("split DNS is not supported by this backend")]
    UnsupportedSplitDns,

    #[error("global DNS is not supported by this backend")]
    UnsupportedGlobalDns,

    #[error("permission denied")]
    PermissionDenied,

    #[error("platform backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Result type for setdns operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;
