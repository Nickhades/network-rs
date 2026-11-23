//! org.freedesktop.NetworkManager (the manager object).

use std::collections::HashMap;
use std::sync::Arc;

use nm_common::consts::{connectivity, manager_state};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, fdo, interface};

use crate::auth;
use crate::core::{Core, CoreError};
use crate::dbus::{SettingsMap, root_path};
use crate::settings::owned;

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
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<OwnedObjectPath> {
        auth::require_admin(bus, header, "ActivateConnection").await?;
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
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let owner_uid = auth::require_admin_uid(bus, header, "AddAndActivateConnection").await?;
        self.core
            .add_and_activate(&connection, &device, &specific_object, Some(owner_uid))
            .await
            .map_err(Into::into)
    }

    async fn add_and_activate_connection2(
        &self,
        connection: SettingsMap,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
        _options: HashMap<String, OwnedVariant>,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedVariant>,
    )> {
        let owner_uid = auth::require_admin_uid(bus, header, "AddAndActivateConnection2").await?;
        let (connection_path, active_path) = self
            .core
            .add_and_activate(&connection, &device, &specific_object, Some(owner_uid))
            .await?;
        // The FreeBSD backend keeps profiles in its own state directory and
        // cannot promise an on-disk NetworkManager keyfile for every caller.
        // Report the standard "volatile" persistence result.
        let mut result = HashMap::new();
        result.insert("persist".to_string(), owned("volatile"));
        Ok((connection_path, active_path, result))
    }

    async fn deactivate_connection(
        &self,
        active_connection: OwnedObjectPath,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "DeactivateConnection").await?;
        self.core
            .deactivate(&active_connection)
            .await
            .map_err(Into::into)
    }

    async fn enable(
        &self,
        enable: bool,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Enable").await?;
        self.core.set_networking_enabled(enable).await;
        Ok(())
    }

    async fn sleep(
        &self,
        sleep: bool,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Sleep").await?;
        self.core.set_networking_enabled(!sleep).await;
        Ok(())
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

    async fn get_permissions(
        &self,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<HashMap<String, String>> {
        auth::permissions(bus, header).await
    }

    async fn set_logging(
        &self,
        level: &str,
        domains: &str,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "SetLogging").await?;
        tracing::debug!(level, domains, "SetLogging requested");
        Ok(())
    }

    async fn reload(
        &self,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Reload").await?;
        tracing::debug!("Reload requested");
        Ok(())
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
