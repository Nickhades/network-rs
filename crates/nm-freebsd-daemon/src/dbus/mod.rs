//! D-Bus interfaces served on the system bus.

pub mod access_point;
pub mod active_connection;
pub mod device;
pub mod dhcp;
pub mod ip4_config;
pub mod manager;
pub mod settings;

use std::collections::HashMap;

use zbus::{Connection, ObjectServer, OwnedObjectPath, OwnedVariant};

pub use crate::settings::SettingsMap;

pub use access_point::AccessPointIface;
pub use active_connection::ActiveConnectionIface;
pub use device::{DeviceIface, WiredIface, WirelessIface};
pub use dhcp::DhcpIface;
pub use ip4_config::Ip4ConfigIface;
pub use manager::ManagerIface;
pub use settings::{SettingsConnectionIface, SettingsIface};

/// Handles registered once the bus connection is up.
#[derive(Clone)]
pub struct Bus {
    pub connection: Connection,
    pub server: ObjectServer,
}

pub fn root_path() -> OwnedObjectPath {
    OwnedObjectPath::try_from("/").expect("root object path")
}

pub fn path_or_root(path: Option<OwnedObjectPath>) -> OwnedObjectPath {
    path.unwrap_or_else(root_path)
}

/// A SettingsMap value used by tests and a few defaults.
pub fn empty_settings() -> SettingsMap {
    HashMap::new()
}

/// Convenience for building the property dictionaries used by IP4Config.
pub fn dict(entries: &[(&str, OwnedVariant)]) -> HashMap<String, OwnedVariant> {
    entries
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_string(),
                value.try_clone().expect("clonable variant"),
            )
        })
        .collect()
}
