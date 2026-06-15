use std::process::{Command, ExitStatus};

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::Error;

#[derive(Debug, thiserror::Error)]
pub(crate) enum PowerShellError {
    #[error("failed to launch PowerShell for {operation}: {source}")]
    Launch {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "PowerShell failed for {operation} with status {status}: stdout: {stdout}; stderr: \
         {stderr}"
    )]
    Status {
        operation: &'static str,
        status: Status,
        stdout: OutputText,
        stderr: OutputText,
    },
}

impl From<PowerShellError> for Error {
    fn from(error: PowerShellError) -> Self {
        Self::Backend(Box::new(error))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OutputText(Vec<u8>);

impl std::fmt::Display for OutputText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&String::from_utf8_lossy(&self.0))
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct Status(ExitStatus);

impl std::fmt::Display for Status {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.code() {
            Some(code) => write!(formatter, "exit code {code}"),
            None => formatter.write_str("terminated by signal"),
        }
    }
}

pub(crate) fn run(operation: &'static str, script: &str) -> Result<Vec<u8>, PowerShellError> {
    log::debug!("running PowerShell for {operation}");
    let encoded = encode_command(script);
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            &encoded,
        ])
        .output()
        .map_err(|source| PowerShellError::Launch { operation, source })?;
    if output.status.success() {
        log::debug!(
            "PowerShell completed {operation}: stdout={} bytes, stderr={} bytes",
            output.stdout.len(),
            output.stderr.len()
        );
        Ok(output.stdout)
    } else {
        log::warn!(
            "PowerShell failed during {operation}: {}",
            Status(output.status)
        );
        Err(PowerShellError::Status {
            operation,
            status: Status(output.status),
            stdout: OutputText(output.stdout),
            stderr: OutputText(output.stderr),
        })
    }
}

fn encode_command(script: &str) -> String {
    let mut bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    STANDARD.encode(bytes)
}
