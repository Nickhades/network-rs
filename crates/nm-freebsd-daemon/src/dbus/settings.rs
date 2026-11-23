//! org.freedesktop.NetworkManager.Settings and Settings.Connection.

use std::sync::Arc;

use zbus::object_server::SignalEmitter;
use zbus::{OwnedObjectPath, fdo, interface};

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

    async fn add_connection(&self, connection: SettingsMap) -> fdo::Result<OwnedObjectPath> {
        self.core
            .add_connection_from_settings(connection, true)
            .await
            .map_err(Into::into)
    }

    async fn add_connection_unsaved(
        &self,
        connection: SettingsMap,
    ) -> fdo::Result<OwnedObjectPath> {
        self.core
            .add_connection_from_settings(connection, false)
            .await
            .map_err(Into::into)
    }

    async fn update_connection(
        &self,
        connection: OwnedObjectPath,
        settings: SettingsMap,
    ) -> fdo::Result<bool> {
        self.core
            .update_connection_settings(&connection, settings, true)
            .await
            .map_err(Into::into)
    }

    async fn delete_connection(&self, connection: OwnedObjectPath) -> fdo::Result<()> {
        self.core
            .delete_connection(&connection)
            .await
            .map_err(Into::into)
    }

    async fn reload_connections(&self) -> fdo::Result<bool> {
        self.core.reload_connections().await.map_err(Into::into)
    }

    async fn save_hostname(&self, hostname: &str) -> fdo::Result<()> {
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

    async fn get_secrets(&self, _service: &str) -> fdo::Result<SettingsMap> {
        self.core
            .connection_secrets_by_uuid(&self.uuid)
            .await
            .map_err(Into::into)
    }

    async fn update(&self, settings: SettingsMap) -> fdo::Result<()> {
        self.core
            .update_connection_by_uuid(&self.uuid, settings, true)
            .await
            .map_err(Into::into)
    }

    async fn update_unsaved(&self, settings: SettingsMap) -> fdo::Result<()> {
        self.core
            .update_connection_by_uuid(&self.uuid, settings, false)
            .await
            .map_err(Into::into)
    }

    async fn delete(&self) -> fdo::Result<()> {
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
