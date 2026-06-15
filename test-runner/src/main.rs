mod env_guard;
mod scenario;
mod system_dns;
mod test_dns;
mod test_tun;
mod verifier;

use std::{net::IpAddr, process::ExitCode};

use clap::{Parser, ValueEnum};
use scenario::{ExitKind, ScenarioOptions};

#[derive(Parser)]
struct Cli {
    mode: Mode,

    #[arg(long)]
    tun: bool,

    #[arg(long = "parent-dns")]
    parent_dns: Option<IpAddr>,

    #[arg(long = "bypass-environment-guard")]
    bypass_environment_guard: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Global,
    Split,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    if let Err(error) = env_guard::check(cli.bypass_environment_guard) {
        tracing::error!("{error:#}");
        return exit_code(ExitKind::Unsupported);
    }

    let options = ScenarioOptions {
        mode: cli.mode.into(),
        tun: cli.tun,
        parent_dns: cli.parent_dns,
    };

    match scenario::run(options).await {
        Ok(()) => exit_code(ExitKind::Pass),
        Err(error) => {
            tracing::error!("{:#}", error.source);
            exit_code(error.kind)
        },
    }
}

fn exit_code(kind: ExitKind) -> ExitCode {
    ExitCode::from(match kind {
        ExitKind::Pass => 0,
        ExitKind::AssertionFailed => 1,
        ExitKind::Unsupported => 2,
        ExitKind::RestoreFailed => 3,
    })
}

impl From<Mode> for scenario::Mode {
    fn from(value: Mode) -> Self {
        match value {
            Mode::Global => Self::Global,
            Mode::Split => Self::Split,
        }
    }
}
