#![doc = include_str!("../README.md")]

mod config;
mod error;
mod platform;

pub use config::Config;
pub use error::{Error, Result};

/// Applied DNS configuration handle.
///
/// The DNS configuration stays active while this value is alive. Call
/// [`SetDns::close`] to restore the previous state and receive any restore
/// error. Dropping the handle also attempts restoration, but drop-time errors
/// can only be logged.
#[must_use = "the DNS configuration is restored when the handle is dropped"]
pub struct SetDns(Option<platform::SetDns>);

impl SetDns {
    /// Validate and apply a DNS configuration.
    ///
    /// Returns [`Error::InvalidConfig`] before touching the platform backend if
    /// the configuration is malformed. Platform, permission, D-Bus,
    /// SystemConfiguration, PowerShell, and I/O failures are returned as
    /// [`Error::Backend`].
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
    ///
    /// Calling `close` is preferred over relying on `Drop` because this method
    /// reports restore failures to the caller.
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
