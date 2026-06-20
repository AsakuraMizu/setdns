use std::{mem, net::IpAddr};

use libloading::os::windows::Library;
use windows::{
    Win32::{
        Foundation::{NO_ERROR, WIN32_ERROR},
        NetworkManagement::{
            IpHelper::{
                ConvertInterfaceAliasToLuid, ConvertInterfaceLuidToGuid, DNS_INTERFACE_SETTINGS,
                DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6, DNS_SETTING_NAMESERVER,
            },
            Ndis::NET_LUID_LH,
        },
    },
    core::{GUID, PCWSTR, PWSTR},
};
use windows_registry::LOCAL_MACHINE;

use super::braced_guid;
use crate::{Error, Result};

pub(crate) struct InterfaceDns {
    snapshots: InterfaceSnapshots,
}

impl InterfaceDns {
    pub(crate) fn apply(alias: &str, servers: &[IpAddr]) -> Result<Option<Self>> {
        let (ipv4, ipv6) = nameservers_by_family(servers);
        let Some(iphlpapi) = Iphlpapi::load()? else {
            log::debug!(
                "SetInterfaceDnsSettings is unavailable; keeping Windows interface DNS unchanged"
            );
            return Ok(None);
        };

        let guid = interface_guid(alias)?;
        let guid_string = braced_guid(&guid);
        let snapshots = InterfaceSnapshots::read(&guid_string, !ipv4.is_empty(), !ipv6.is_empty())?;

        if let Err(error) = iphlpapi.set_dns(alias, guid, &ipv4, &ipv6) {
            if let Err(restore_error) = snapshots.restore() {
                log::warn!(
                    "failed to restore Windows interface DNS after apply failure: {restore_error}"
                );
            }
            return Err(error);
        }

        Ok(Some(Self { snapshots }))
    }

    pub(crate) fn close(self) -> Result<()> {
        self.snapshots.restore()
    }
}

#[derive(Debug, thiserror::Error)]
enum InterfaceError {
    #[error("failed to load iphlpapi.dll: {0}")]
    LoadLibrary(String),
    #[error("failed to resolve Windows interface alias '{alias}': Win32 error {code}")]
    ResolveAlias { alias: String, code: u32 },
    #[error("failed to convert Windows interface alias '{alias}' to GUID: Win32 error {code}")]
    ConvertGuid { alias: String, code: u32 },
    #[error("failed to read or restore Windows interface DNS registry state: {0}")]
    Registry(String),
    #[error(
        "failed to set {family} DNS servers on Windows interface '{alias}': Win32 error {code}"
    )]
    SetDns {
        alias: String,
        family: &'static str,
        code: u32,
    },
}

impl From<InterfaceError> for Error {
    fn from(error: InterfaceError) -> Self {
        Self::Backend(Box::new(error))
    }
}

type SetInterfaceDnsSettingsFn =
    unsafe extern "system" fn(GUID, *const DNS_INTERFACE_SETTINGS) -> WIN32_ERROR;

const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

struct Iphlpapi {
    _library: Library,
    set_interface_dns_settings: SetInterfaceDnsSettingsFn,
}

impl Iphlpapi {
    fn load() -> Result<Option<Self>> {
        let library =
            match unsafe { Library::load_with_flags("iphlpapi.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }
            {
                Ok(library) => library,
                Err(error) => {
                    log::debug!("failed to load iphlpapi.dll from System32 directly: {error}");
                    unsafe { Library::new("iphlpapi.dll") }
                        .map_err(|error| InterfaceError::LoadLibrary(error.to_string()))?
                },
            };
        let set_interface_dns_settings = {
            let symbol =
                unsafe { library.get::<SetInterfaceDnsSettingsFn>(b"SetInterfaceDnsSettings\0") };
            match symbol {
                Ok(symbol) => *symbol,
                Err(error) => {
                    log::debug!("SetInterfaceDnsSettings symbol is unavailable: {error}");
                    return Ok(None);
                },
            }
        };

        Ok(Some(Self {
            _library: library,
            set_interface_dns_settings,
        }))
    }

    fn set_dns(&self, alias: &str, guid: GUID, ipv4: &str, ipv6: &str) -> Result<()> {
        if !ipv4.is_empty() {
            self.set_dns_family(alias, guid, AddressFamily::Ipv4, ipv4)?;
        }
        if !ipv6.is_empty() {
            self.set_dns_family(alias, guid, AddressFamily::Ipv6, ipv6)?;
        }
        Ok(())
    }

    fn set_dns_family(
        &self,
        alias: &str,
        guid: GUID,
        family: AddressFamily,
        nameservers: &str,
    ) -> Result<()> {
        let mut nameservers_wide = utf16_null_terminated(nameservers);
        let settings = DNS_INTERFACE_SETTINGS {
            Version: DNS_INTERFACE_SETTINGS_VERSION1,
            Flags: family.flags(),
            NameServer: PWSTR(nameservers_wide.as_mut_ptr()),
            ..Default::default()
        };
        let error = unsafe { (self.set_interface_dns_settings)(guid, &settings) };
        if error != NO_ERROR {
            return Err(InterfaceError::SetDns {
                alias: alias.to_owned(),
                family: family.label(),
                code: error.0,
            }
            .into());
        }

        Ok(())
    }
}

fn interface_guid(alias: &str) -> Result<GUID> {
    let alias_wide = utf16_null_terminated(alias);
    let mut luid: NET_LUID_LH = unsafe { mem::zeroed() };
    let error = unsafe { ConvertInterfaceAliasToLuid(PCWSTR(alias_wide.as_ptr()), &mut luid) };
    if error != NO_ERROR {
        return Err(InterfaceError::ResolveAlias {
            alias: alias.to_owned(),
            code: error.0,
        }
        .into());
    }

    let mut guid = GUID::zeroed();
    let error = unsafe { ConvertInterfaceLuidToGuid(&luid, &mut guid) };
    if error != NO_ERROR {
        return Err(InterfaceError::ConvertGuid {
            alias: alias.to_owned(),
            code: error.0,
        }
        .into());
    }

    Ok(guid)
}

struct InterfaceSnapshots {
    ipv4: Option<NameServerSnapshot>,
    ipv6: Option<NameServerSnapshot>,
}

impl InterfaceSnapshots {
    fn read(guid: &str, has_ipv4: bool, has_ipv6: bool) -> Result<Self> {
        Ok(Self {
            ipv4: has_ipv4
                .then(|| NameServerSnapshot::read(guid, AddressFamily::Ipv4))
                .transpose()?,
            ipv6: has_ipv6
                .then(|| NameServerSnapshot::read(guid, AddressFamily::Ipv6))
                .transpose()?,
        })
    }

    fn restore(&self) -> Result<()> {
        if let Some(snapshot) = &self.ipv4 {
            snapshot.restore()?;
        }
        if let Some(snapshot) = &self.ipv6 {
            snapshot.restore()?;
        }
        Ok(())
    }
}

struct NameServerSnapshot {
    path: String,
    value: Option<String>,
}

impl NameServerSnapshot {
    fn read(guid: &str, family: AddressFamily) -> Result<Self> {
        let path = format!("{}\\{}", family.registry_base(), guid);
        let value = read_nameserver(&path)?;
        Ok(Self { path, value })
    }

    fn restore(&self) -> Result<()> {
        restore_nameserver(&self.path, self.value.as_deref())
    }
}

const IPV4_INTERFACE_BASE: &str = r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces";
const IPV6_INTERFACE_BASE: &str = r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces";

#[derive(Clone, Copy)]
enum AddressFamily {
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    fn label(self) -> &'static str {
        match self {
            Self::Ipv4 => "IPv4",
            Self::Ipv6 => "IPv6",
        }
    }

    fn registry_base(self) -> &'static str {
        match self {
            Self::Ipv4 => IPV4_INTERFACE_BASE,
            Self::Ipv6 => IPV6_INTERFACE_BASE,
        }
    }

    fn flags(self) -> u64 {
        let flags = u64::from(DNS_SETTING_NAMESERVER);
        match self {
            Self::Ipv4 => flags,
            Self::Ipv6 => flags | u64::from(DNS_SETTING_IPV6),
        }
    }
}

fn nameservers_by_family(servers: &[IpAddr]) -> (String, String) {
    let mut ipv4 = String::new();
    let mut ipv6 = String::new();

    for server in servers {
        match server {
            IpAddr::V4(_) => push_nameserver(&mut ipv4, server),
            IpAddr::V6(_) => push_nameserver(&mut ipv6, server),
        }
    }

    (ipv4, ipv6)
}

fn push_nameserver(nameservers: &mut String, server: &IpAddr) {
    if !nameservers.is_empty() {
        nameservers.push(',');
    }
    nameservers.push_str(&server.to_string());
}

fn utf16_null_terminated(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn read_nameserver(path: &str) -> Result<Option<String>> {
    let key = LOCAL_MACHINE
        .open(path)
        .map_err(|error| InterfaceError::Registry(error.to_string()))?;
    Ok(key.get_string("NameServer").ok())
}

fn restore_nameserver(path: &str, value: Option<&str>) -> Result<()> {
    let key = LOCAL_MACHINE
        .create(path)
        .map_err(|error| InterfaceError::Registry(error.to_string()))?;
    match value {
        Some(value) => key
            .set_string("NameServer", value)
            .map_err(|error| InterfaceError::Registry(error.to_string()))?,
        None => {
            if let Err(error) = key.remove_value("NameServer") {
                log::debug!("failed to remove absent Windows interface NameServer value: {error}");
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::{nameservers_by_family, utf16_null_terminated};

    #[test]
    fn builds_only_non_empty_address_families() {
        let (ipv4, ipv6) = nameservers_by_family(&[
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        ]);

        assert_eq!(ipv4, "1.1.1.1,8.8.8.8");
        assert_eq!(ipv6, "::1");
    }

    #[test]
    fn skips_ipv6_when_no_ipv6_server_is_configured() {
        let (ipv4, ipv6) = nameservers_by_family(&[IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))]);

        assert_eq!(ipv4, "1.1.1.1");
        assert_eq!(ipv6, "");
    }

    #[test]
    fn encodes_nameserver_as_null_terminated_utf16() {
        assert_eq!(
            utf16_null_terminated("1.1.1.1"),
            vec![49, 46, 49, 46, 49, 46, 49, 0]
        );
    }
}
