mod config;
mod error;

pub use config::Config;
pub use error::{Error, Result};

/// RAII handle that restores DNS configuration when closed or dropped.
pub struct SetDns(Option<InnerSetDns>);

impl SetDns {
    /// Validate and apply a DNS configuration.
    pub fn apply(config: Config) -> Result<Self> {
        let config = config.normalize()?;

        if config.domains.is_empty() {
            Err(Error::UnsupportedGlobalDns)
        } else {
            Err(Error::UnsupportedSplitDns)
        }
    }

    /// Restore the previous DNS configuration and consume this handle.
    pub fn close(mut self) -> Result<()> {
        if let Some(inner) = self.0.take() {
            inner.close()
        } else {
            Ok(())
        }
    }
}

impl Drop for SetDns {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take()
            && let Err(err) = inner.close()
        {
            log::warn!("failed to restore DNS configuration for setdns: {err}");
        }
    }
}

struct InnerSetDns;

impl InnerSetDns {
    fn close(self) -> Result<()> {
        Ok(())
    }
}
