use crate::{Error, Result, config::NormalizedConfig};

pub(crate) struct SetDns;

#[derive(Debug, thiserror::Error)]
enum UnsupportedPlatformError {
    #[error("global DNS is not supported on this platform")]
    GlobalDns,
    #[error("split DNS is not supported on this platform")]
    SplitDns,
}

impl From<UnsupportedPlatformError> for Error {
    fn from(error: UnsupportedPlatformError) -> Self {
        Self::Backend(Box::new(error))
    }
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        if config.domains.is_empty() {
            log::debug!("global DNS is unsupported on this platform");
            Err(UnsupportedPlatformError::GlobalDns.into())
        } else {
            log::debug!("split DNS is unsupported on this platform");
            Err(UnsupportedPlatformError::SplitDns.into())
        }
    }

    pub(crate) fn close(self) -> Result<()> {
        Ok(())
    }
}
