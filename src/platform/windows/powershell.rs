use std::process::{Command, ExitStatus};

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
        Ok(output.stdout)
    } else {
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
    base64(&bytes)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);

        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::encode_command;

    #[test]
    fn encodes_powershell_command_as_utf16le_base64() {
        assert_eq!(
            encode_command("Write-Output 'ok'"),
            "VwByAGkAdABlAC0ATwB1AHQAcAB1AHQAIAAnAG8AawAnAA=="
        );
    }
}
