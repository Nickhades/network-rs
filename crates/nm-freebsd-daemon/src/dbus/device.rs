//! Device, Device.Wireless and Device.Wired interfaces.

use std::sync::Arc;

use nm_common::consts::{device_capability, device_type, wifi_device_capability, wireless_mode};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedObjectPath, fdo, interface};

use crate::auth;
use crate::core::Core;
use crate::dbus::{SettingsMap, root_path};

pub struct DeviceIface {
    core: Arc<Core>,
    ifname: String,
}

impl DeviceIface {
    pub fn new(core: Arc<Core>, ifname: String) -> Self {
        Self { core, ifname }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Device")]
impl DeviceIface {
    #[zbus(property)]
    async fn udi(&self) -> String {
        format!("/virtual/net/{}", self.ifname)
    }

    #[zbus(property)]
    async fn interface(&self) -> String {
        self.ifname.clone()
    }

    #[zbus(property)]
    async fn ip_interface(&self) -> String {
        self.ifname.clone()
    }

    #[zbus(property)]
    async fn driver(&self) -> String {
        "unknown".to_string()
    }

    #[zbus(property)]
    async fn driver_version(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    async fn firmware_version(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    async fn capabilities(&self) -> u32 {
        device_capability::NM_SUPPORTED
    }

    #[zbus(property)]
    async fn ip4_address(&self) -> u32 {
        let state = self.core.state.read().await;
        state
            .devices
            .get(&self.ifname)
            .and_then(|device| device.ip4.as_ref())
            .and_then(|ip4| ip4.address.parse::<std::net::Ipv4Addr>().ok())
            .map(u32::from)
            .unwrap_or(0)
    }

    #[zbus(property(emits_changed_signal = "false"), name = "State")]
    async fn device_state(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.state)
            .unwrap_or(nm_common::consts::device_state::UNKNOWN)
    }

    #[zbus(property)]
    async fn state_reason(&self) -> (u32, u32) {
        let state = self.core.state.read().await;
        state
            .devices
            .get(&self.ifname)
            .map(|device| (device.state, device.state_reason))
            .unwrap_or((nm_common::consts::device_state::UNKNOWN, 0))
    }

    #[zbus(property)]
    async fn active_connection(&self) -> OwnedObjectPath {
        let state = self.core.state.read().await;
        state
            .devices
            .get(&self.ifname)
            .and_then(|device| device.active_connection.clone())
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn available_connections(&self) -> Vec<OwnedObjectPath> {
        let state = self.core.state.read().await;
        let Some(device) = state.devices.get(&self.ifname) else {
            return Vec::new();
        };
        state
            .connections
            .values()
            .filter(|profile| {
                profile.kind == crate::model::ProfileKind::Wifi && device.kind.is_wifi()
                    || profile.kind == crate::model::ProfileKind::Ethernet && !device.kind.is_wifi()
            })
            .map(|profile| profile.path.clone())
            .collect()
    }

    #[zbus(property)]
    async fn autoconnect(&self) -> bool {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.autoconnect)
            .unwrap_or(false)
    }

    #[zbus(property)]
    async fn managed(&self) -> bool {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.managed)
            .unwrap_or(false)
    }

    #[zbus(property)]
    async fn device_type(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.kind.nm_type())
            .unwrap_or(device_type::UNKNOWN)
    }

    #[zbus(property)]
    async fn dhcp4_config(&self) -> OwnedObjectPath {
        root_path()
    }

    #[zbus(property)]
    async fn ip4_config(&self) -> OwnedObjectPath {
        let state = self.core.state.read().await;
        state
            .devices
            .get(&self.ifname)
            .and_then(|device| device.ip4_config_path.clone())
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn ip6_config(&self) -> OwnedObjectPath {
        root_path()
    }

    #[zbus(property)]
    async fn mtu(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.mtu)
            .unwrap_or(0)
    }

    async fn reapply(
        &self,
        _connection: SettingsMap,
        _version_id: u64,
        _flags: u32,
    ) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "Reapply is not implemented by the FreeBSD MVP".to_string(),
        ))
    }

    async fn delete(&self) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "deleting a FreeBSD interface is not supported".to_string(),
        ))
    }

    async fn disconnect(
        &self,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Device.Disconnect").await?;
        self.core
            .disconnect_device(&self.ifname)
            .await
            .map_err(Into::into)
    }

    #[zbus(signal)]
    pub async fn state_changed(
        emitter: &SignalEmitter<'_>,
        new_state: u32,
        old_state: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

pub struct WirelessIface {
    core: Arc<Core>,
    ifname: String,
}

impl WirelessIface {
    pub fn new(core: Arc<Core>, ifname: String) -> Self {
        Self { core, ifname }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl WirelessIface {
    async fn get_access_points(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.ap_paths(&self.ifname)
    }

    async fn get_all_access_points(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.ap_paths(&self.ifname)
    }

    async fn request_scan(
        &self,
        _options: std::collections::HashMap<String, zbus::OwnedVariant>,
    ) -> fdo::Result<()> {
        self.core
            .request_scan(&self.ifname)
            .await
            .map_err(Into::into)
    }

    #[zbus(property)]
    async fn hw_address(&self) -> String {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.mac.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn perm_hw_address(&self) -> String {
        self.hw_address().await
    }

    #[zbus(property)]
    async fn mode(&self) -> u32 {
        wireless_mode::INFRA
    }

    #[zbus(property)]
    async fn bitrate(&self) -> u32 {
        0
    }

    #[zbus(property)]
    async fn access_points(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.ap_paths(&self.ifname)
    }

    #[zbus(property)]
    async fn active_access_point(&self) -> OwnedObjectPath {
        let state = self.core.state.read().await;
        state
            .devices
            .get(&self.ifname)
            .and_then(|device| device.active_ap.clone())
            .and_then(|key| state.aps.get(&key).map(|ap| ap.path.clone()))
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn wireless_capabilities(&self) -> u32 {
        wifi_device_capability::CIPHER_CCMP
            | wifi_device_capability::RSN
            | wifi_device_capability::WPA
            | wifi_device_capability::CIPHER_TKIP
    }

    #[zbus(property)]
    async fn last_scan(&self) -> i64 {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.last_scan)
            .unwrap_or(0)
    }

    #[zbus(signal)]
    pub async fn access_point_added(
        emitter: &SignalEmitter<'_>,
        ap: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn access_point_removed(
        emitter: &SignalEmitter<'_>,
        ap: OwnedObjectPath,
    ) -> zbus::Result<()>;
}

pub struct WiredIface {
    core: Arc<Core>,
    ifname: String,
}

impl WiredIface {
    pub fn new(core: Arc<Core>, ifname: String) -> Self {
        Self { core, ifname }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Device.Wired")]
impl WiredIface {
    #[zbus(property)]
    async fn hw_address(&self) -> String {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.mac.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn perm_hw_address(&self) -> String {
        self.hw_address().await
    }

    #[zbus(property)]
    async fn speed(&self) -> u32 {
        0
    }

    #[zbus(property)]
    async fn carrier(&self) -> bool {
        self.core
            .state
            .read()
            .await
            .devices
            .get(&self.ifname)
            .map(|device| device.state != nm_common::consts::device_state::UNAVAILABLE)
            .unwrap_or(false)
    }
}
