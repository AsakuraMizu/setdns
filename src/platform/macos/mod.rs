mod global;
mod resolver;
pub(crate) mod state;

use crate::{Result, config::NormalizedConfig};

pub(crate) enum SetDns {
    Global(global::SetDns),
    Resolver(resolver::SetDns),
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        if config.domains.is_empty() {
            global::SetDns::apply(config).map(Self::Global)
        } else {
            resolver::SetDns::apply(config).map(Self::Resolver)
        }
    }

    pub(crate) fn close(self) -> Result<()> {
        match self {
            Self::Global(inner) => inner.close(),
            Self::Resolver(inner) => inner.close(),
        }
    }
}
