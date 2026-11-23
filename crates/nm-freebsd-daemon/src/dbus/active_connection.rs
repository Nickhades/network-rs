//! org.freedesktop.NetworkManager.Connection.Active.

use std::sync::Arc;

use zbus::{OwnedObjectPath, interface};

use crate::core::Core;
use crate::dbus::root_path;

pub struct ActiveConnectionIface {
    core: Arc<Core>,
    path: OwnedObjectPath,
}

impl ActiveConnectionIface {
    pub fn new(core: Arc<Core>, path: OwnedObjectPath) -> Self {
        Self { core, path }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl ActiveConnectionIface {
    #[zbus(property)]
    async fn connection(&self) -> OwnedObjectPath {
        self.core
            .state
            .read()
            .await
            .active
            .get(&self.path)
            .map(|active| active.connection_path.clone())
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn specific_object(&self) -> OwnedObjectPath {
        self.core
            .state
            .read()
            .await
            .active
            .get(&self.path)
            .map(|active| active.specific_object.clone())
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn id(&self) -> String {
        self.core
            .state
            .read()
            .await
            .active
            .get(&self.path)
            .map(|active| active.id.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn uuid(&self) -> String {
        self.core
            .state
            .read()
            .await
            .active
            .get(&self.path)
            .map(|active| active.uuid.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Type")]
    async fn connection_type(&self) -> String {
        let state = self.core.state.read().await;
        state
            .active
            .get(&self.path)
            .and_then(|active| state.connections.get(&active.connection_path))
            .map(|profile| profile.kind.connection_type().to_string())
            .unwrap_or_else(|| "802-11-wireless".to_string())
    }

    #[zbus(property)]
    async fn devices(&self) -> Vec<OwnedObjectPath> {
        self.core
            .state
            .read()
            .await
            .active
            .get(&self.path)
            .map(|active| vec![active.device_path.clone()])
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn state(&self) -> u32 {
        self.core
            .state
            .read()
            .await
            .active
            .get(&self.path)
            .map(|active| active.state)
            .unwrap_or(nm_common::consts::active_connection_state::UNKNOWN)
    }

    #[zbus(property)]
    async fn state_flags(&self) -> u32 {
        0
    }

    #[zbus(property)]
    async fn default(&self) -> bool {
        self.core
            .state
            .read()
            .await
            .active
            .get(&self.path)
            .map(|active| active.default)
            .unwrap_or(false)
    }

    #[zbus(property, name = "Default6")]
    async fn default6(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn vpn(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn master(&self) -> OwnedObjectPath {
        root_path()
    }

    #[zbus(property)]
    async fn ip4_config(&self) -> OwnedObjectPath {
        let state = self.core.state.read().await;
        state
            .active
            .get(&self.path)
            .and_then(|active| active.ip4_config_path.clone())
            .unwrap_or_else(root_path)
    }

    #[zbus(property)]
    async fn dhcp4_config(&self) -> OwnedObjectPath {
        root_path()
    }

    #[zbus(property)]
    async fn ip6_config(&self) -> OwnedObjectPath {
        root_path()
    }

    #[zbus(property)]
    async fn dhcp6_config(&self) -> OwnedObjectPath {
        root_path()
    }
}
