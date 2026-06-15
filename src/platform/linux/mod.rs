mod resolv_conf;
mod resolved;

use crate::{Error, Result, config::NormalizedConfig};

pub(crate) enum SetDns {
    Resolved(resolved::SetDns),
    ResolvConf(resolv_conf::SetDns),
}

#[derive(Debug, thiserror::Error)]
enum LinuxError {
    #[error("global DNS is unsupported: {reason}")]
    UnsupportedGlobalDns { reason: &'static str },
    #[error("split DNS requires systemd-resolved")]
    SplitDnsNeedsResolved {
        #[source]
        source: resolved::ResolvedError,
    },
    #[error("split DNS is unsupported: {reason}")]
    UnsupportedSplitDns { reason: &'static str },
}

impl From<LinuxError> for Error {
    fn from(error: LinuxError) -> Self {
        Self::Backend(Box::new(error))
    }
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        let resolved = resolved::Manager::connect();

        if config.domains.is_empty() {
            if let (Ok(manager), Some(_)) = (&resolved, config.device.as_ref()) {
                log::debug!(
                    "selected Linux systemd-resolved backend for global DNS on device {}",
                    config.device.as_deref().expect("device checked by pattern")
                );
                return manager.apply(config).map(Self::Resolved);
            }

            if resolv_conf::is_managed_by_resolved() {
                log::debug!(
                    "Linux global DNS without device is unsupported because /etc/resolv.conf is \
                     managed by systemd-resolved"
                );
                return Err(LinuxError::UnsupportedGlobalDns {
                    reason: "/etc/resolv.conf is managed by systemd-resolved and no device was \
                             specified",
                }
                .into());
            }

            log::debug!("selected Linux /etc/resolv.conf backend for global DNS");
            return resolv_conf::SetDns::apply(config).map(Self::ResolvConf);
        }

        let manager = resolved.map_err(|source| LinuxError::SplitDnsNeedsResolved { source })?;
        if config.device.is_none() {
            log::debug!("Linux split DNS is unsupported without a device/interface name");
            return Err(LinuxError::UnsupportedSplitDns {
                reason: "Linux split DNS requires a device/interface name",
            }
            .into());
        }

        log::debug!(
            "selected Linux systemd-resolved backend for split DNS on device {}",
            config.device.as_deref().expect("device checked above")
        );
        manager.apply(config).map(Self::Resolved)
    }

    pub(crate) fn close(self) -> Result<()> {
        match self {
            Self::Resolved(inner) => inner.close(),
            Self::ResolvConf(inner) => inner.close(),
        }
    }
}
