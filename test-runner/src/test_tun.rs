use std::net::{IpAddr, Ipv4Addr};

use anyhow::{Context, Result};
use tun::AbstractDevice;

pub const TUN_DNS_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(198, 18, 0, 53));

pub struct TestTun {
    name: String,
    _device: tun::Device,
}

impl TestTun {
    pub fn create() -> Result<Self> {
        let mut config = tun::Configuration::default();
        config
            .address(TUN_DNS_IP)
            .netmask(Ipv4Addr::new(255, 255, 255, 255));
        config.up();

        let device = tun::create(&config).context("failed to create TUN device")?;
        let name = device
            .tun_name()
            .context("failed to read TUN device name")?;

        Ok(Self {
            name,
            _device: device,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
