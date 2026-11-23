//! org.freedesktop.NetworkManager (the manager object).

use std::collections::HashMap;
use std::sync::Arc;

use nm_common::consts::{connectivity, manager_state};
use zbus::object_server::SignalEmitter;
use zbus::{OwnedObjectPath, OwnedVariant, fdo, interface};

use crate::core::{Core, CoreError};
use crate::dbus::{SettingsMap, root_path};

pub struct ManagerIface {
    core: Arc<Core>,
}

impl ManagerIface {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

#[interface(name = "org.freedesktop.NetworkManager")]
impl ManagerIface {
    // ---- properties -----------------------------------------------------

    #[zbus(property)]
    async fn version(&self) -> String {
        // Advertise a recent 1.x API. Frontends gate optional properties on
        // this value, so it must not claim features newer than we implement.
        "1.46.0".to_string()
    }

    #[zbus(property(emits_changed_signal = "false"), name = "State")]
    async fn manager_state(&self) -> u32 {
        self.core.state.read().await.manager_state
    }

    #[zbus(property)]
    async fn startup(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn networking_enabled(&self) -> bool {
        self.core.state.read().await.networking_enabled
    }

    #[zbus(property)]
    async fn wireless_enabled(&self) -> bool {
        self.core.state.read().await.wireless_enabled
    }

    #[zbus(property)]
    async fn set_wireless_enabled(&self, value: bool) {
        self.core.set_wireless_enabled(value).await;
    }

    #[zbus(property)]
    async fn wireless_hardware_enabled(&self) -> bool {
        self.core
            .state
            .read()
            .await
            .devices
            .values()
            .any(|device| device.kind.is_wifi())
    }

    #[zbus(property)]
    async fn connectivity(&self) -> u32 {
        self.core.state.read().await.connectivity
    }

    #[zbus(property)]
    async fn devices(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.device_paths()
    }

    #[zbus(property)]
    async fn active_connections(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.active_paths()
    }

    #[zbus(property)]
    async fn primary_connection(&self) -> OwnedObjectPath {
        self.core
            .state
            .read()
            .await
            .primary_connection
            .clone()
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn activating_connection(&self) -> OwnedObjectPath {
        self.core
            .state
            .read()
            .await
            .active
            .values()
            .find(|active| active.is_activating())
            .map(|active| active.path.clone())
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn metered(&self) -> u32 {
        nm_common::consts::metered::UNKNOWN
    }

    #[zbus(property)]
    async fn global_dns_configuration(&self) -> HashMap<String, OwnedVariant> {
        HashMap::new()
    }

    // ---- methods --------------------------------------------------------

    async fn get_devices(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.device_paths()
    }

    async fn get_all_devices(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.device_paths()
    }

    async fn get_device_by_ip_iface(&self, iface: &str) -> fdo::Result<OwnedObjectPath> {
        self.core
            .state
            .read()
            .await
            .devices
            .get(iface)
            .map(|device| device.path.clone())
            .ok_or_else(|| fdo::Error::UnknownObject(format!("unknown interface {iface}")))
    }

    async fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> fdo::Result<OwnedObjectPath> {
        self.core
            .activate(&connection, &device, &specific_object)
            .await
            .map_err(Into::into)
    }

    async fn add_and_activate_connection(
        &self,
        connection: SettingsMap,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        self.core
            .add_and_activate(&connection, &device, &specific_object)
            .await
            .map_err(Into::into)
    }

    async fn deactivate_connection(&self, active_connection: OwnedObjectPath) -> fdo::Result<()> {
        self.core
            .deactivate(&active_connection)
            .await
            .map_err(Into::into)
    }

    async fn enable(&self, enable: bool) {
        self.core.set_networking_enabled(enable).await;
    }

    async fn sleep(&self, sleep: bool) {
        self.core.set_networking_enabled(!sleep).await;
    }

    async fn check_connectivity(&self) -> u32 {
        let state = self.core.state.read().await;
        if state.connectivity == connectivity::UNKNOWN
            && state.manager_state == manager_state::CONNECTED_GLOBAL
        {
            connectivity::FULL
        } else {
            state.connectivity
        }
    }

    async fn get_permissions(&self) -> HashMap<String, String> {
        // This MVP has no polkit integration. Be explicit rather than
        // pretending to authorize: the D-Bus policy file controls access.
        let mut permissions = HashMap::new();
        for key in [
            "org.freedesktop.NetworkManager.enable-disable-network",
            "org.freedesktop.NetworkManager.enable-disable-wifi",
            "org.freedesktop.NetworkManager.enable-disable-wwan",
            "org.freedesktop.NetworkManager.sleep-wake",
            "org.freedesktop.NetworkManager.wifi-share",
            "org.freedesktop.NetworkManager.wifi-share-open",
            "org.freedesktop.NetworkManager.wifi-scan",
            "org.freedesktop.NetworkManager.network-control",
            "org.freedesktop.NetworkManager.settings-modify-system",
            "org.freedesktop.NetworkManager.settings-modify-own",
            "org.freedesktop.NetworkManager.wifi-connect",
        ] {
            permissions.insert(key.to_string(), "yes".to_string());
        }
        permissions
    }

    async fn set_logging(&self, level: &str, domains: &str) {
        tracing::debug!(level, domains, "SetLogging requested");
    }

    async fn reload(&self) {
        tracing::debug!("Reload requested");
    }

    // ---- signals --------------------------------------------------------
    //
    // The State property disables the macro-generated property-changed
    // helper, because its Rust name would collide with this signal. The
    // PropertiesChanged dictionary is emitted directly from core instead.

    #[zbus(signal)]
    pub async fn state_changed(emitter: &SignalEmitter<'_>, state: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn device_added(
        emitter: &SignalEmitter<'_>,
        device: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn device_removed(
        emitter: &SignalEmitter<'_>,
        device: OwnedObjectPath,
    ) -> zbus::Result<()>;
}

impl From<CoreError> for fdo::Error {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::UnknownObject(path) => fdo::Error::UnknownObject(path),
            CoreError::InvalidArgs(message) => fdo::Error::InvalidArgs(message),
            CoreError::NotSupported(message) => fdo::Error::NotSupported(message),
            other => fdo::Error::Failed(other.to_string()),
        }
    }
}
