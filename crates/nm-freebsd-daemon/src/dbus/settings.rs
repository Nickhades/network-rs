//! org.freedesktop.NetworkManager.Settings and Settings.Connection.

use std::sync::Arc;

use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedObjectPath, fdo, interface};

use crate::auth;
use crate::core::Core;
use crate::dbus::SettingsMap;

pub struct SettingsIface {
    core: Arc<Core>,
}

impl SettingsIface {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Settings")]
impl SettingsIface {
    #[zbus(property)]
    async fn hostname(&self) -> String {
        self.core.state.read().await.hostname.clone()
    }

    #[zbus(property)]
    async fn can_modify(&self) -> bool {
        true
    }

    async fn list_connections(&self) -> Vec<OwnedObjectPath> {
        self.core.state.read().await.connection_paths()
    }

    async fn get_connection(&self, connection: OwnedObjectPath) -> fdo::Result<SettingsMap> {
        self.core
            .connection_settings(&connection, false)
            .await
            .map_err(Into::into)
    }

    async fn add_connection(
        &self,
        connection: SettingsMap,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<OwnedObjectPath> {
        auth::require_admin(bus, header, "AddConnection").await?;
        self.core
            .add_connection_from_settings(connection, true)
            .await
            .map_err(Into::into)
    }

    async fn add_connection_unsaved(
        &self,
        connection: SettingsMap,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<OwnedObjectPath> {
        auth::require_admin(bus, header, "AddConnectionUnsaved").await?;
        self.core
            .add_connection_from_settings(connection, false)
            .await
            .map_err(Into::into)
    }

    async fn update_connection(
        &self,
        connection: OwnedObjectPath,
        settings: SettingsMap,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<bool> {
        auth::require_admin(bus, header, "UpdateConnection").await?;
        self.core
            .update_connection_settings(&connection, settings, true)
            .await
            .map_err(Into::into)
    }

    async fn delete_connection(
        &self,
        connection: OwnedObjectPath,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "DeleteConnection").await?;
        self.core
            .delete_connection(&connection)
            .await
            .map_err(Into::into)
    }

    async fn reload_connections(
        &self,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<bool> {
        auth::require_admin(bus, header, "ReloadConnections").await?;
        self.core.reload_connections().await.map_err(Into::into)
    }

    async fn save_hostname(
        &self,
        hostname: &str,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "SaveHostname").await?;
        self.core.set_hostname(hostname).await;
        Ok(())
    }

    #[zbus(signal)]
    pub async fn new_connection(
        emitter: &SignalEmitter<'_>,
        connection: OwnedObjectPath,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn connection_removed(
        emitter: &SignalEmitter<'_>,
        connection: OwnedObjectPath,
    ) -> zbus::Result<()>;
}

pub struct SettingsConnectionIface {
    core: Arc<Core>,
    uuid: String,
}

impl SettingsConnectionIface {
    pub fn new(core: Arc<Core>, uuid: String) -> Self {
        Self { core, uuid }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl SettingsConnectionIface {
    async fn get_settings(&self) -> fdo::Result<SettingsMap> {
        self.core
            .connection_settings_by_uuid(&self.uuid, false)
            .await
            .map_err(Into::into)
    }

    async fn get_secrets(
        &self,
        _service: &str,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<SettingsMap> {
        auth::require_admin(bus, header, "GetSecrets").await?;
        self.core
            .connection_secrets_by_uuid(&self.uuid)
            .await
            .map_err(Into::into)
    }

    async fn update(
        &self,
        settings: SettingsMap,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Settings.Update").await?;
        self.core
            .update_connection_by_uuid(&self.uuid, settings, true)
            .await
            .map_err(Into::into)
    }

    async fn update_unsaved(
        &self,
        settings: SettingsMap,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Settings.UpdateUnsaved").await?;
        self.core
            .update_connection_by_uuid(&self.uuid, settings, false)
            .await
            .map_err(Into::into)
    }

    async fn delete(
        &self,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Settings.Delete").await?;
        self.core
            .delete_connection_by_uuid(&self.uuid)
            .await
            .map_err(Into::into)
    }

    #[zbus(signal)]
    pub async fn updated(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn removed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
