mod powershell;

use std::{fmt::Write as _, net::IpAddr};

use crate::{Error, Result, config::NormalizedConfig};

const RULE_NAME_PREFIX: &str = "__SETDNS_NRPT_NAME__";
const NAMESPACE_CHUNK_SIZE: usize = 50;

pub(crate) struct SetDns {
    rule_names: Vec<String>,
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        let _ = &config.device;

        let namespaces = namespaces(&config);
        let script = apply_script(&config.owner, &config.servers, &namespaces);
        let stdout = powershell::run("apply NRPT rules", &script).map_err(backend_error)?;
        let rule_names = parse_rule_names(&stdout);

        Ok(Self { rule_names })
    }

    pub(crate) fn close(self) -> Result<()> {
        let script = close_script(&self.rule_names);
        powershell::run("remove NRPT rules", &script).map_err(backend_error)?;
        Ok(())
    }
}

fn backend_error(error: powershell::PowerShellError) -> Error {
    Error::Backend(Box::new(error))
}

fn namespaces(config: &NormalizedConfig) -> Vec<String> {
    if config.domains.is_empty() {
        return vec![".".to_owned()];
    }

    config
        .domains
        .iter()
        .map(|domain| format!(".{}", domain.domain))
        .collect()
}

fn apply_script(owner: &str, servers: &[IpAddr], namespaces: &[String]) -> String {
    let mut script = String::new();
    write_header(&mut script, owner);
    write_string_array(&mut script, "$NameServers", &server_strings(servers));
    write_namespace_chunks(&mut script, namespaces);
    script.push_str(
        r#"
$oldRules = @(Get-DnsClientNrptRule -ErrorAction Stop | Where-Object { $_.DisplayName -eq $Owner })
foreach ($rule in $oldRules) {
    if ($null -ne $rule.Name -and $rule.Name -ne '') {
        Remove-DnsClientNrptRule -Name $rule.Name -Force -ErrorAction Stop
    }
}

$createdNames = @()
foreach ($chunk in $NamespaceChunks) {
    $rules = @(Add-DnsClientNrptRule -Namespace $chunk -NameServers $NameServers -DisplayName $Owner -PassThru -ErrorAction Stop)
    foreach ($rule in $rules) {
        if ($null -ne $rule.Name -and $rule.Name -ne '') {
            $createdNames += [string]$rule.Name
        }
    }
}

if ($createdNames.Count -ne $NamespaceChunks.Count) {
    throw "Add-DnsClientNrptRule did not return a rule name for every namespace chunk."
}

try { Clear-DnsClientCache -ErrorAction Stop } catch { }
foreach ($name in $createdNames) {
    Write-Output ("__SETDNS_NRPT_NAME__" + $name)
}
"#,
    );
    script
}

fn close_script(rule_names: &[String]) -> String {
    let mut script = String::new();
    script.push_str("$ErrorActionPreference = 'Stop'\n");
    write_string_array(&mut script, "$RuleNames", rule_names);
    script.push_str(
        r#"
foreach ($name in $RuleNames) {
    Remove-DnsClientNrptRule -Name $name -Force -ErrorAction Stop
}

try { Clear-DnsClientCache -ErrorAction Stop } catch { }
"#,
    );
    script
}

fn write_header(script: &mut String, owner: &str) {
    script.push_str("$ErrorActionPreference = 'Stop'\n");
    writeln!(script, "$Owner = {}", powershell_string(owner)).expect("write to String cannot fail");
}

fn write_namespace_chunks(script: &mut String, namespaces: &[String]) {
    script.push_str("$NamespaceChunks = @(\n");
    for chunk in namespaces.chunks(NAMESPACE_CHUNK_SIZE) {
        script.push_str("    , ");
        write_array_literal(script, chunk);
        script.push('\n');
    }
    script.push_str(")\n");
}

fn write_string_array(script: &mut String, name: &str, values: &[String]) {
    script.push_str(name);
    script.push_str(" = ");
    write_array_literal(script, values);
    script.push('\n');
}

fn write_array_literal(script: &mut String, values: &[String]) {
    script.push_str("@(");
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            script.push_str(", ");
        }
        script.push_str(&powershell_string(value));
    }
    script.push(')');
}

fn powershell_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if character == '\'' {
            quoted.push('\'');
        }
        quoted.push(character);
    }
    quoted.push('\'');
    quoted
}

fn server_strings(servers: &[IpAddr]) -> Vec<String> {
    servers.iter().map(|server| server.to_string()).collect()
}

fn parse_rule_names(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| line.strip_prefix(RULE_NAME_PREFIX).map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::{NAMESPACE_CHUNK_SIZE, apply_script, close_script, namespaces, parse_rule_names};
    use crate::config::{DomainSuffix, NormalizedConfig};

    #[test]
    fn maps_global_namespace_to_dot() {
        let config = config(Vec::new());

        assert_eq!(namespaces(&config), vec!["."]);
    }

    #[test]
    fn maps_split_namespaces_to_dot_prefixed_suffixes() {
        let config = config(vec![
            DomainSuffix {
                domain: "corp.internal".to_owned(),
                wildcard: false,
            },
            DomainSuffix {
                domain: "example.net".to_owned(),
                wildcard: true,
            },
        ]);

        assert_eq!(namespaces(&config), vec![".corp.internal", ".example.net"]);
    }

    #[test]
    fn chunks_split_namespaces_by_fifty() {
        let namespaces: Vec<String> = (0..=NAMESPACE_CHUNK_SIZE)
            .map(|index| format!(".{index}.example"))
            .collect();
        let script = apply_script("owner", &[IpAddr::V4(Ipv4Addr::LOCALHOST)], &namespaces);

        assert!(script.contains("$NamespaceChunks = @(\n    , @('.0.example'"));
        assert!(script.contains("\n    , @('.50.example')\n)"));
    }

    #[test]
    fn close_script_removes_only_recorded_rule_names() {
        let script = close_script(&["rule-one".to_owned(), "rule-two".to_owned()]);

        assert!(script.contains("$RuleNames = @('rule-one', 'rule-two')"));
        assert!(script.contains("Remove-DnsClientNrptRule -Name $name -Force"));
        assert!(!script.contains("Get-DnsClientNrptRule"));
    }

    #[test]
    fn parses_only_tagged_rule_names() {
        let stdout = b"ignored\n__SETDNS_NRPT_NAME__rule-one\n__SETDNS_NRPT_NAME__rule-two\n";

        assert_eq!(parse_rule_names(stdout), vec!["rule-one", "rule-two"]);
    }

    fn config(domains: Vec<DomainSuffix>) -> NormalizedConfig {
        NormalizedConfig {
            owner: "owner".to_owned(),
            servers: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            domains,
            device: None,
        }
    }
}
