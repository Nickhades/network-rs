//! org.freedesktop.NetworkManager.AccessPoint.

use std::sync::Arc;

use nm_common::consts::wireless_mode;
use zbus::interface;

use crate::core::Core;

pub struct AccessPointIface {
    core: Arc<Core>,
    key: String,
}

impl AccessPointIface {
    pub fn new(core: Arc<Core>, ifname: String, bssid: String) -> Self {
        Self {
            core,
            key: crate::model::ApEntry::ap_key(&ifname, &bssid),
        }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPointIface {
    #[zbus(property)]
    async fn flags(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.flags)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn wpa_flags(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.wpa_flags)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn rsn_flags(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.rsn_flags)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn ssid(&self) -> Vec<u8> {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.ssid.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn frequency(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.frequency)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn hw_address(&self) -> String {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.bssid.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn mode(&self) -> u32 {
        wireless_mode::INFRA
    }

    #[zbus(property)]
    async fn max_bitrate(&self) -> u32 {
        0
    }

    #[zbus(property)]
    async fn strength(&self) -> u8 {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.strength)
            .unwrap_or(0)
    }

    #[zbus(property)]
    async fn last_seen(&self) -> i32 {
        self.core
            .state
            .read()
            .await
            .aps
            .get(&self.key)
            .map(|ap| ap.last_seen)
            .unwrap_or(-1)
    }
}
