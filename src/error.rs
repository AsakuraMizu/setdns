/// Error returned by configuration validation or a platform DNS backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The supplied [`crate::Config`] failed validation before any platform
    /// backend was called.
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    /// The selected platform backend failed while applying or restoring DNS.
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

/// Result alias used by this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
