//! Internal DHCP hook notification interface.
//!
//! This is not a NetworkManager standard interface. It exists so the root
//! dhclient exit hook can report lease events without scraping a lease file.
//! The policy does not allow unprivileged callers; the method also checks the
//! caller uid.

use std::collections::HashMap;
use std::sync::Arc;

use zbus::message::Header;
use zbus::{Connection, fdo, interface};

use crate::auth;
use crate::core::Core;

pub struct DhcpIface {
    core: Arc<Core>,
}

impl DhcpIface {
    pub fn new(core: Arc<Core>) -> Self {
        Self { core }
    }
}

#[interface(name = "org.freedesktop.NetworkManager.Dhcp")]
impl DhcpIface {
    /// Called by /etc/dhclient-exit-hooks via /usr/local/libexec/nm-notify-dhcp.
    async fn notify(
        &self,
        ifname: &str,
        reason: &str,
        data: HashMap<String, String>,
        #[zbus(connection)] bus: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> fdo::Result<()> {
        auth::require_admin(bus, header, "Dhcp.Notify").await?;
        self.core.handle_dhcp_event(ifname, reason, &data).await;
        Ok(())
    }
}
