mod config;
mod error;
mod platform;

pub use config::Config;
pub use error::{Error, Result};

/// RAII handle that restores DNS configuration when closed or dropped.
pub struct SetDns(Option<platform::SetDns>);

impl SetDns {
    /// Validate and apply a DNS configuration.
    pub fn apply(config: Config) -> Result<Self> {
        platform::SetDns::apply(config.normalize()?).map(|inner| Self(Some(inner)))
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
