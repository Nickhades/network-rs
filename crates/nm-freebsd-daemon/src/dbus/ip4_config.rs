//! org.freedesktop.NetworkManager.IP4Config.
//!
//! The object reads the live device address, so a DHCP lease change is visible
//! without recreating the object.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;

use zbus::{OwnedVariant, interface};

use crate::core::Core;
use crate::model::Ip4Info;
use crate::settings::owned;

pub struct Ip4ConfigIface {
    core: Arc<Core>,
    ifname: String,
}

impl Ip4ConfigIface {
    pub fn new(core: Arc<Core>, ifname: String) -> Self {
        Self { core, ifname }
    }

    async fn info(&self) -> Ip4Info {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .and_then(|device| device.ip4.clone())
            .unwrap_or_default()
    }
}

#[interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl Ip4ConfigIface {
    #[zbus(property)]
    async fn address_data(&self) -> Vec<HashMap<String, OwnedVariant>> {
        let info = self.info().await;
        if info.address.is_empty() {
            return Vec::new();
        }
        vec![HashMap::from([
            ("address".to_string(), owned(info.address)),
            ("prefix".to_string(), owned(u32::from(info.prefix))),
        ])]
    }

    #[zbus(property)]
    async fn addresses(&self) -> Vec<Vec<u32>> {
        let info = self.info().await;
        info.address
            .parse::<Ipv4Addr>()
            .ok()
            .map(|address| vec![vec![u32::from(address), u32::from(info.prefix)]])
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn gateway(&self) -> String {
        self.info().await.gateway.unwrap_or_default()
    }

    #[zbus(property)]
    async fn nameserver_data(&self) -> Vec<HashMap<String, OwnedVariant>> {
        self.info()
            .await
            .dns
            .iter()
            .map(|server| HashMap::from([("address".to_string(), owned(server.clone()))]))
            .collect()
    }

    #[zbus(property)]
    async fn nameservers(&self) -> Vec<u32> {
        self.info()
            .await
            .dns
            .iter()
            .filter_map(|server| server.parse::<Ipv4Addr>().ok())
            .map(u32::from)
            .collect()
    }

    #[zbus(property)]
    async fn domains(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    async fn routes(&self) -> Vec<HashMap<String, OwnedVariant>> {
        Vec::new()
    }

    #[zbus(property)]
    async fn dns_options(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    async fn dns_priority(&self) -> i32 {
        0
    }
}
