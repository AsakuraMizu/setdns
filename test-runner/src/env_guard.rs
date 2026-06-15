use std::{env, path::Path};

use anyhow::{Result, bail};

pub fn check(bypass: bool) -> Result<()> {
    if bypass || recognized_ci() || recognized_container() {
        return Ok(());
    }

    bail!(
        "refusing to run outside a recognized CI or container environment; pass \
         --bypass-environment-guard only if you explicitly accept disabling this guard"
    )
}

fn recognized_ci() -> bool {
    matches!(env::var("CI"), Ok(value) if is_truthy(&value))
        || env::var_os("GITHUB_ACTIONS").is_some()
        || env::var_os("GITLAB_CI").is_some()
        || env::var_os("BUILDKITE").is_some()
        || env::var_os("TF_BUILD").is_some()
        || env::var_os("JENKINS_URL").is_some()
}

fn recognized_container() -> bool {
    Path::new("/.dockerenv").exists()
        || Path::new("/run/.containerenv").exists()
        || env::var_os("KUBERNETES_SERVICE_HOST").is_some()
        || matches!(env::var("container"), Ok(value) if !value.is_empty())
}

fn is_truthy(value: &str) -> bool {
    !matches!(value, "" | "0" | "false" | "FALSE" | "False")
}
