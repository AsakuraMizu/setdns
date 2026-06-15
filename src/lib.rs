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
        let config = config.normalize()?;
        log::debug!(
            "applying DNS configuration: mode={}, servers={}, domains={}, device={}",
            if config.domains.is_empty() {
                "global"
            } else {
                "split"
            },
            config.servers.len(),
            config.domains.len(),
            config.device.as_deref().unwrap_or("default")
        );
        let inner = platform::SetDns::apply(config)?;
        log::info!("applied DNS configuration");
        Ok(Self(Some(inner)))
    }

    /// Restore the previous DNS configuration and consume this handle.
    pub fn close(mut self) -> Result<()> {
        if let Some(inner) = self.0.take() {
            inner.close()?;
            log::info!("restored DNS configuration");
        }
        Ok(())
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
