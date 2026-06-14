use crate::{Error, Result, config::NormalizedConfig};

pub(crate) struct SetDns;

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        if config.domains.is_empty() {
            Err(Error::UnsupportedGlobalDns)
        } else {
            Err(Error::UnsupportedSplitDns)
        }
    }

    pub(crate) fn close(self) -> Result<()> {
        Ok(())
    }
}
