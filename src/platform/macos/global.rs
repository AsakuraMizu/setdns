use std::{net::IpAddr, ptr};

use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType, ToVoid},
    dictionary::{CFDictionary, CFMutableDictionary},
    string::CFString,
};
use core_foundation_sys::{
    base::{Boolean, CFRelease},
    dictionary::{CFDictionaryRef, CFDictionarySetValue},
};
use system_configuration::{
    dynamic_store::SCDynamicStoreBuilder,
    network_configuration::{SCNetworkService, SCNetworkSet},
    preferences::SCPreferences,
};
use system_configuration_sys::{
    network_configuration::{
        SCNetworkInterfaceForceConfigurationRefresh, SCNetworkProtocolGetConfiguration,
        SCNetworkProtocolRef, SCNetworkProtocolSetConfiguration, SCNetworkServiceCopy,
        SCNetworkServiceCopyProtocol, SCNetworkServiceGetEnabled,
    },
    preferences::{SCPreferencesApplyChanges, SCPreferencesLock, SCPreferencesUnlock},
    schema_definitions::{
        kSCDynamicStorePropNetPrimaryService, kSCEntNetDNS, kSCPropNetDNSServerAddresses,
    },
};

use crate::{Error, Result, config::NormalizedConfig};

const STORE_NAME: &str = "setdns";
const PRIMARY_IPV4_KEY: &str = "State:/Network/Global/IPv4";

pub(crate) struct SetDns {
    service_id: String,
    original_dns: Option<CFDictionary>,
}

#[derive(Debug, thiserror::Error)]
enum GlobalError {
    #[error("no current network set")]
    NoCurrentSet,
    #[error("no primary network service")]
    NoPrimaryService,
    #[error("network service {0} was not found")]
    ServiceNotFound(String),
    #[error("network service {0} has no DNS protocol")]
    NoDnsProtocol(String),
    #[error("failed to lock SystemConfiguration preferences")]
    LockFailed,
    #[error("failed to unlock SystemConfiguration preferences")]
    UnlockFailed,
    #[error("failed to set DNS protocol configuration for service {0}")]
    SetConfigurationFailed(String),
    #[error("failed to apply SystemConfiguration changes")]
    ApplyFailed,
}

impl SetDns {
    pub(crate) fn apply(config: NormalizedConfig) -> Result<Self> {
        let prefs = preferences();
        let service = target_service(&prefs, config.device.as_deref())?;
        let service_id = service_id(&service)?;
        let protocol = dns_protocol(&service, &service_id)?;
        let original_dns = protocol_configuration(&protocol);
        let next_dns = dns_with_servers(original_dns.as_ref(), &config.servers);

        with_preferences_lock(&prefs, || {
            set_protocol_configuration(&protocol, Some(&next_dns), &service_id)?;
            apply_changes(&prefs)
        })?;

        refresh_interface(&service);
        crate::platform::macos::state::flush_dns_cache();

        Ok(Self {
            service_id,
            original_dns,
        })
    }

    pub(crate) fn close(self) -> Result<()> {
        let prefs = preferences();
        let service = service_by_id(&prefs, &self.service_id)?;
        let protocol = dns_protocol(&service, &self.service_id)?;

        with_preferences_lock(&prefs, || {
            set_protocol_configuration(&protocol, self.original_dns.as_ref(), &self.service_id)?;
            apply_changes(&prefs)
        })?;

        refresh_interface(&service);
        crate::platform::macos::state::flush_dns_cache();
        Ok(())
    }
}

fn preferences() -> SCPreferences {
    SCPreferences::default(&CFString::new("setdns"))
}

fn target_service(prefs: &SCPreferences, device: Option<&str>) -> Result<SCNetworkService> {
    if let Some(ifname) = device {
        return service_by_interface(prefs, ifname);
    }

    let service_id = primary_service_id()?;
    service_by_id(prefs, &service_id)
}

fn service_by_interface(prefs: &SCPreferences, ifname: &str) -> Result<SCNetworkService> {
    let set = current_set(prefs)?;
    let services = SCNetworkService::get_services(prefs);

    for ordered_id in set.service_order().iter() {
        let ordered_id = ordered_id.to_string();
        for service in services.iter() {
            if service_id_ref(&service).as_deref() != Some(ordered_id.as_str()) {
                continue;
            }
            if !service_enabled(&service) {
                continue;
            }
            let Some(interface) = service.network_interface() else {
                continue;
            };
            let Some(bsd_name) = interface.bsd_name() else {
                continue;
            };
            if bsd_name == ifname {
                return Ok(service.clone());
            }
        }
    }

    Err(backend(GlobalError::ServiceNotFound(ifname.to_owned())))
}

fn service_by_id(prefs: &SCPreferences, service_id: &str) -> Result<SCNetworkService> {
    let cf_id = CFString::new(service_id);
    let service = unsafe { SCNetworkServiceCopy(prefs.to_void(), cf_id.as_concrete_TypeRef()) };
    if service.is_null() {
        Err(backend(GlobalError::ServiceNotFound(service_id.to_owned())))
    } else {
        Ok(unsafe { SCNetworkService::wrap_under_create_rule(service) })
    }
}

fn current_set(prefs: &SCPreferences) -> Result<SCNetworkSet> {
    let set = unsafe {
        system_configuration_sys::network_configuration::SCNetworkSetCopyCurrent(prefs.to_void())
    };
    if set.is_null() {
        Err(backend(GlobalError::NoCurrentSet))
    } else {
        Ok(unsafe { SCNetworkSet::wrap_under_create_rule(set) })
    }
}

fn primary_service_id() -> Result<String> {
    let Some(store) = SCDynamicStoreBuilder::new(CFString::new(STORE_NAME)).build() else {
        return Err(backend(GlobalError::NoPrimaryService));
    };
    let Some(value) = store.get(PRIMARY_IPV4_KEY) else {
        return Err(backend(GlobalError::NoPrimaryService));
    };
    if !value.instance_of::<CFDictionary>() {
        return Err(backend(GlobalError::NoPrimaryService));
    }
    let dictionary = unsafe {
        CFDictionary::<CFString, CFType>::wrap_under_get_rule(
            value.as_concrete_TypeRef() as CFDictionaryRef
        )
    };
    let key = unsafe { CFString::wrap_under_get_rule(kSCDynamicStorePropNetPrimaryService) };
    dictionary
        .find(&key)
        .and_then(|service| service.downcast::<CFString>())
        .map(|service| service.to_string())
        .ok_or_else(|| backend(GlobalError::NoPrimaryService))
}

struct DnsProtocol {
    raw: SCNetworkProtocolRef,
}

impl DnsProtocol {
    fn as_ptr(&self) -> SCNetworkProtocolRef {
        self.raw
    }
}

impl Drop for DnsProtocol {
    fn drop(&mut self) {
        unsafe { CFRelease(self.raw as _) };
    }
}

fn dns_protocol(service: &SCNetworkService, service_id: &str) -> Result<DnsProtocol> {
    let protocol =
        unsafe { SCNetworkServiceCopyProtocol(service.as_concrete_TypeRef(), kSCEntNetDNS) };
    if protocol.is_null() {
        Err(backend(GlobalError::NoDnsProtocol(service_id.to_owned())))
    } else {
        Ok(DnsProtocol { raw: protocol })
    }
}

fn protocol_configuration(protocol: &DnsProtocol) -> Option<CFDictionary> {
    let configuration = unsafe { SCNetworkProtocolGetConfiguration(protocol.as_ptr()) };
    if configuration.is_null() {
        None
    } else {
        Some(unsafe { CFDictionary::wrap_under_get_rule(configuration) })
    }
}

fn dns_with_servers(original: Option<&CFDictionary>, servers: &[IpAddr]) -> CFDictionary {
    let dictionary: CFMutableDictionary = match original {
        Some(original) => unsafe { original.to_mutable() }.copy_with_capacity(0),
        None => CFMutableDictionary::new(),
    };
    let key = unsafe { CFString::wrap_under_get_rule(kSCPropNetDNSServerAddresses) };
    let values: Vec<CFString> = servers
        .iter()
        .map(|server| CFString::new(&server.to_string()))
        .collect();
    let addresses = CFArray::from_CFTypes(&values);
    unsafe {
        CFDictionarySetValue(
            dictionary.as_concrete_TypeRef(),
            key.to_void(),
            addresses.as_concrete_TypeRef() as _,
        );
    }
    dictionary.to_immutable()
}

fn set_protocol_configuration(
    protocol: &DnsProtocol,
    configuration: Option<&CFDictionary>,
    service_id: &str,
) -> Result<()> {
    let config = configuration.map_or(ptr::null(), |dns| dns.as_concrete_TypeRef());
    if unsafe { SCNetworkProtocolSetConfiguration(protocol.as_ptr(), config) } == 0 {
        Err(backend(GlobalError::SetConfigurationFailed(
            service_id.to_owned(),
        )))
    } else {
        Ok(())
    }
}

fn with_preferences_lock<F>(prefs: &SCPreferences, f: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    if unsafe { SCPreferencesLock(prefs.to_void(), true as Boolean) } == 0 {
        return Err(backend(GlobalError::LockFailed));
    }

    let result = f();
    let unlock_result = if unsafe { SCPreferencesUnlock(prefs.to_void()) } == 0 {
        Err(backend(GlobalError::UnlockFailed))
    } else {
        Ok(())
    };

    result.and(unlock_result)
}

fn apply_changes(prefs: &SCPreferences) -> Result<()> {
    if unsafe { SCPreferencesApplyChanges(prefs.to_void()) } == 0 {
        Err(backend(GlobalError::ApplyFailed))
    } else {
        Ok(())
    }
}

fn refresh_interface(service: &SCNetworkService) {
    if let Some(interface) = service.network_interface() {
        let _ =
            unsafe { SCNetworkInterfaceForceConfigurationRefresh(interface.as_concrete_TypeRef()) };
    }
}

fn service_enabled(service: &SCNetworkService) -> bool {
    unsafe { SCNetworkServiceGetEnabled(service.as_concrete_TypeRef()) != 0 }
}

fn service_id(service: &SCNetworkService) -> Result<String> {
    service_id_ref(service).ok_or_else(|| backend(GlobalError::NoPrimaryService))
}

fn service_id_ref(service: &SCNetworkService) -> Option<String> {
    service.id().map(|id| id.to_string())
}

fn backend(error: GlobalError) -> Error {
    Error::Backend(Box::new(error))
}
