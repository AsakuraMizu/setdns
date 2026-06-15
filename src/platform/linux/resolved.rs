use std::{ffi::CString, net::IpAddr};

use zbus::blocking::{Connection, Proxy};

use crate::{Error, Result, config::NormalizedConfig};

const DESTINATION: &str = "org.freedesktop.resolve1";
const PATH: &str = "/org/freedesktop/resolve1";
const MANAGER_INTERFACE: &str = "org.freedesktop.resolve1.Manager";

pub(crate) struct Manager {
    connection: Connection,
}

pub(crate) struct SetDns {
    connection: Connection,
    ifindex: i32,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ResolvedError {
    #[error("failed to connect to the system D-Bus")]
    SystemBus(#[source] zbus::Error),
    #[error("systemd-resolved is not available on the system D-Bus")]
    ServiceUnavailable(#[source] zbus::Error),
    #[error("systemd-resolved has no owner on the system D-Bus")]
    NoOwner,
    #[error("interface name contains an interior NUL byte")]
    InvalidInterfaceName(#[source] std::ffi::NulError),
    #[error("failed to resolve interface index for {interface}")]
    InterfaceIndex {
        interface: String,
        #[source]
        source: std::io::Error,
    },
    #[error("systemd-resolved D-Bus call {method} failed")]
    Method {
        method: &'static str,
        #[source]
        source: zbus::Error,
    },
}

impl From<ResolvedError> for Error {
    fn from(error: ResolvedError) -> Self {
        Self::Backend(Box::new(error))
    }
}

impl Manager {
    pub(crate) fn connect() -> std::result::Result<Self, ResolvedError> {
        let connection = Connection::system().map_err(ResolvedError::SystemBus)?;
        let dbus = Proxy::new(
            &connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .map_err(ResolvedError::ServiceUnavailable)?;
        let has_owner: bool = dbus
            .call("NameHasOwner", &(DESTINATION))
            .map_err(ResolvedError::ServiceUnavailable)?;
        if !has_owner {
            return Err(ResolvedError::NoOwner);
        }

        log::debug!("connected to systemd-resolved on the system bus");
        Ok(Self { connection })
    }

    pub(crate) fn apply(&self, config: NormalizedConfig) -> Result<SetDns> {
        let ifindex = interface_index(
            config
                .device
                .as_deref()
                .expect("device is checked by caller"),
        )?;
        log::debug!(
            "applying systemd-resolved DNS: ifindex={}, mode={}, servers={}, domains={}",
            ifindex,
            if config.domains.is_empty() {
                "global"
            } else {
                "split"
            },
            config.servers.len(),
            config.domains.len()
        );
        let proxy = manager_proxy(&self.connection)?;
        let dns = dns_servers(&config.servers);
        call(&proxy, "SetLinkDNS", &(ifindex, dns))?;

        if config.domains.is_empty() {
            let domains = [(".", true)];
            call(&proxy, "SetLinkDomains", &(ifindex, domains.as_slice()))?;
            call(&proxy, "SetLinkDefaultRoute", &(ifindex, true))?;
        } else {
            let domains: Vec<(String, bool)> = config
                .domains
                .iter()
                .map(|suffix| (resolved_domain(&suffix.domain), true))
                .collect();
            call(&proxy, "SetLinkDomains", &(ifindex, domains.as_slice()))?;
            call(&proxy, "SetLinkDefaultRoute", &(ifindex, false))?;
        }

        call(&proxy, "FlushCaches", &())?;
        log::debug!("flushed systemd-resolved caches after apply");

        Ok(SetDns {
            connection: self.connection.clone(),
            ifindex,
        })
    }
}

impl SetDns {
    pub(crate) fn close(self) -> Result<()> {
        let proxy = manager_proxy(&self.connection)?;
        call(&proxy, "RevertLink", &(self.ifindex))?;
        log::debug!(
            "reverted systemd-resolved link configuration: ifindex={}",
            self.ifindex
        );
        call(&proxy, "FlushCaches", &())
    }
}

fn manager_proxy(connection: &Connection) -> Result<Proxy<'_>> {
    Ok(Proxy::new(connection, DESTINATION, PATH, MANAGER_INTERFACE)
        .map_err(ResolvedError::ServiceUnavailable)?)
}

fn call<B>(proxy: &Proxy<'_>, method: &'static str, body: &B) -> Result<()>
where
    B: serde::ser::Serialize + zbus::zvariant::DynamicType,
{
    proxy
        .call::<_, _, ()>(method, body)
        .map_err(|source| ResolvedError::Method { method, source })?;
    Ok(())
}

fn interface_index(interface: &str) -> Result<i32> {
    let name = CString::new(interface).map_err(ResolvedError::InvalidInterfaceName)?;
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 {
        return Err(ResolvedError::InterfaceIndex {
            interface: interface.to_owned(),
            source: std::io::Error::last_os_error(),
        }
        .into());
    }
    Ok(index as i32)
}

fn dns_servers(servers: &[IpAddr]) -> Vec<(i32, Vec<u8>)> {
    servers
        .iter()
        .map(|server| match server {
            IpAddr::V4(addr) => (libc::AF_INET, addr.octets().to_vec()),
            IpAddr::V6(addr) => (libc::AF_INET6, addr.octets().to_vec()),
        })
        .collect()
}

fn resolved_domain(domain: &str) -> String {
    let mut resolved = String::with_capacity(domain.len() + 1);
    resolved.push_str(domain);
    resolved.push('.');
    resolved
}
