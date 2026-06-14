mod resolv_conf;
mod resolved;

use crate::{Error, Result, config::NormalizedConfig};

pub(crate) enum SetDns {
    Resolved(resolved::SetDns),
    ResolvConf(resolv_conf::SetDns),
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        let resolved = resolved::Manager::connect().ok();

        if config.domains.is_empty() {
            if let (Some(manager), Some(_)) = (&resolved, config.device.as_ref()) {
                return manager.apply(config).map(Self::Resolved);
            }

            if resolv_conf::is_managed_by_resolved() {
                return Err(Error::UnsupportedGlobalDns);
            }

            return resolv_conf::SetDns::apply(config).map(Self::ResolvConf);
        }

        let Some(manager) = resolved else {
            return Err(Error::UnsupportedSplitDns);
        };
        if config.device.is_none() {
            return Err(Error::UnsupportedSplitDns);
        }

        manager.apply(config).map(Self::Resolved)
    }

    pub(crate) fn close(self) -> Result<()> {
        match self {
            Self::Resolved(inner) => inner.close(),
            Self::ResolvConf(inner) => inner.close(),
        }
    }
}
