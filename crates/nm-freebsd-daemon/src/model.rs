//! In-memory model shared by the D-Bus interface objects and the backends.

use std::collections::HashMap;

use nm_common::consts::{
    active_connection_state, device_state, device_state_reason, device_type, manager_state,
};
use nm_common::names;
use nm_common::security::ApSecurity;
use zbus::OwnedObjectPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Ethernet,
    Wifi,
    Other,
}

impl DeviceKind {
    pub fn nm_type(self) -> u32 {
        match self {
            Self::Ethernet => device_type::ETHERNET,
            Self::Wifi => device_type::WIFI,
            Self::Other => device_type::GENERIC,
        }
    }

    pub fn is_wifi(self) -> bool {
        matches!(self, Self::Wifi)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Ip4Info {
    pub address: String,
    pub prefix: u8,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceEntry {
    pub ifname: String,
    pub ifindex: i32,
    pub kind: DeviceKind,
    pub mac: String,
    pub mtu: u32,
    pub state: u32,
    pub state_reason: u32,
    pub path: OwnedObjectPath,
    pub managed: bool,
    pub autoconnect: bool,
    pub ip4: Option<Ip4Info>,
    pub ip4_config_path: Option<OwnedObjectPath>,
    pub active_ap: Option<String>,
    pub active_connection: Option<OwnedObjectPath>,
    pub last_scan: i64,
}

impl DeviceEntry {
    pub fn new(ifname: String, ifindex: i32, kind: DeviceKind, mac: String, mtu: u32) -> Self {
        let path = OwnedObjectPath::try_from(format!("{}/{}", names::NM_DEVICES_PATH, ifindex))
            .expect("valid device path");
        let managed = kind != DeviceKind::Other;
        Self {
            ifname,
            ifindex,
            kind,
            mac,
            mtu,
            state: if managed {
                device_state::DISCONNECTED
            } else {
                device_state::UNMANAGED
            },
            state_reason: device_state_reason::NONE,
            path,
            managed,
            autoconnect: true,
            ip4: None,
            ip4_config_path: None,
            active_ap: None,
            active_connection: None,
            last_scan: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApEntry {
    pub ifname: String,
    pub bssid: String,
    pub ssid: Vec<u8>,
    pub frequency: u32,
    pub strength: u8,
    pub flags: u32,
    pub wpa_flags: u32,
    pub rsn_flags: u32,
    pub last_seen: i32,
    pub path: OwnedObjectPath,
    pub security: ApSecurity,
}

impl ApEntry {
    pub fn ap_key(ifname: &str, bssid: &str) -> String {
        format!("{ifname}/{bssid}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileKind {
    Wifi,
    Ethernet,
}

impl ProfileKind {
    pub fn connection_type(self) -> &'static str {
        match self {
            Self::Wifi => "802-11-wireless",
            Self::Ethernet => "802-3-ethernet",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectionProfile {
    pub path: OwnedObjectPath,
    pub uuid: String,
    pub id: String,
    pub kind: ProfileKind,
    pub ifname: Option<String>,
    pub ssid: Vec<u8>,
    pub psk: Option<String>,
    pub key_mgmt: Option<String>,
    pub hidden: bool,
    pub autoconnect: bool,
    pub timestamp: u64,
}

impl ConnectionProfile {
    pub fn is_wifi(&self) -> bool {
        self.kind == ProfileKind::Wifi
    }
}

#[derive(Debug, Clone)]
pub struct ActiveEntry {
    pub path: OwnedObjectPath,
    pub connection_path: OwnedObjectPath,
    pub device_path: OwnedObjectPath,
    pub specific_object: OwnedObjectPath,
    pub id: String,
    pub uuid: String,
    pub state: u32,
    pub default: bool,
    pub ip4_config_path: Option<OwnedObjectPath>,
}

impl ActiveEntry {
    pub fn is_activated(&self) -> bool {
        self.state == active_connection_state::ACTIVATED
    }

    pub fn is_activating(&self) -> bool {
        self.state == active_connection_state::ACTIVATING
    }
}

#[derive(Debug)]
pub struct CoreState {
    /// Devices keyed by interface name.
    pub devices: HashMap<String, DeviceEntry>,
    /// APs keyed by "ifname/bssid".
    pub aps: HashMap<String, ApEntry>,
    pub connections: HashMap<OwnedObjectPath, ConnectionProfile>,
    pub active: HashMap<OwnedObjectPath, ActiveEntry>,
    pub manager_state: u32,
    pub connectivity: u32,
    pub networking_enabled: bool,
    pub wireless_enabled: bool,
    pub primary_connection: Option<OwnedObjectPath>,
    pub hostname: String,
    next_ap_id: u32,
    next_connection_id: u32,
    next_active_id: u32,
    next_ip4_id: u32,
}

impl CoreState {
    pub fn new(hostname: String) -> Self {
        Self {
            devices: HashMap::new(),
            aps: HashMap::new(),
            connections: HashMap::new(),
            active: HashMap::new(),
            manager_state: manager_state::DISCONNECTED,
            connectivity: nm_common::consts::connectivity::UNKNOWN,
            networking_enabled: true,
            wireless_enabled: true,
            primary_connection: None,
            hostname,
            next_ap_id: 1,
            next_connection_id: 1,
            next_active_id: 1,
            next_ip4_id: 1,
        }
    }

    pub fn next_ap_path(&mut self) -> OwnedObjectPath {
        let id = self.next_ap_id;
        self.next_ap_id += 1;
        OwnedObjectPath::try_from(format!("{}/{}", names::NM_ACCESS_POINTS_PATH, id))
            .expect("valid AP path")
    }

    pub fn next_connection_path(&mut self) -> OwnedObjectPath {
        let id = self.next_connection_id;
        self.next_connection_id += 1;
        OwnedObjectPath::try_from(format!("{}/{}", names::NM_SETTINGS_PATH, id))
            .expect("valid connection path")
    }

    pub fn next_active_path(&mut self) -> OwnedObjectPath {
        let id = self.next_active_id;
        self.next_active_id += 1;
        OwnedObjectPath::try_from(format!("{}/{}", names::NM_ACTIVE_CONNECTIONS_PATH, id))
            .expect("valid active connection path")
    }

    pub fn next_ip4_path(&mut self) -> OwnedObjectPath {
        let id = self.next_ip4_id;
        self.next_ip4_id += 1;
        OwnedObjectPath::try_from(format!("{}/{}", names::NM_IP4_CONFIG_PATH, id))
            .expect("valid IP4Config path")
    }

    pub fn device_paths(&self) -> Vec<OwnedObjectPath> {
        let mut paths: Vec<_> = self.devices.values().map(|d| d.path.clone()).collect();
        paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        paths
    }

    pub fn device_by_path(&self, path: &OwnedObjectPath) -> Option<&DeviceEntry> {
        self.devices.values().find(|d| &d.path == path)
    }

    pub fn device_path_for_ifname(&self, ifname: &str) -> Option<OwnedObjectPath> {
        self.devices.get(ifname).map(|d| d.path.clone())
    }

    pub fn wifi_devices(&self) -> Vec<&DeviceEntry> {
        let mut devices: Vec<_> = self.devices.values().filter(|d| d.kind.is_wifi()).collect();
        devices.sort_by(|a, b| a.ifname.cmp(&b.ifname));
        devices
    }

    pub fn ap_paths(&self, ifname: &str) -> Vec<OwnedObjectPath> {
        let mut paths: Vec<_> = self
            .aps
            .values()
            .filter(|ap| ap.ifname == ifname)
            .map(|ap| ap.path.clone())
            .collect();
        paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        paths
    }

    pub fn ap_by_path(&self, path: &OwnedObjectPath) -> Option<&ApEntry> {
        self.aps.values().find(|ap| &ap.path == path)
    }

    pub fn ap_by_bssid(&self, ifname: &str, bssid: &str) -> Option<&ApEntry> {
        self.aps.get(&ApEntry::ap_key(ifname, bssid))
    }

    pub fn connection_paths(&self) -> Vec<OwnedObjectPath> {
        let mut paths: Vec<_> = self.connections.keys().cloned().collect();
        paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        paths
    }

    pub fn active_paths(&self) -> Vec<OwnedObjectPath> {
        let mut paths: Vec<_> = self.active.keys().cloned().collect();
        paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        paths
    }

    pub fn active_for_device(&self, device_path: &OwnedObjectPath) -> Option<&ActiveEntry> {
        self.active
            .values()
            .find(|active| &active.device_path == device_path)
    }

    pub fn recompute_manager_state(&mut self) {
        if !self.networking_enabled {
            self.manager_state = manager_state::ASLEEP;
            self.primary_connection = None;
            return;
        }
        if self.active.values().any(|a| a.is_activated()) {
            self.manager_state = manager_state::CONNECTED_GLOBAL;
            self.connectivity = nm_common::consts::connectivity::FULL;
            self.primary_connection = self
                .active
                .values()
                .find(|a| a.is_activated())
                .map(|a| a.path.clone());
        } else if self.active.values().any(|a| a.is_activating()) {
            self.manager_state = manager_state::CONNECTING;
            self.primary_connection = None;
        } else if self.devices.values().any(|device| {
            device.state == device_state::ACTIVATED || device.state == device_state::IP_CONFIG
        }) {
            // A link that wpa_supplicant or DHCP brought up outside this
            // daemon, or a link whose DHCP lease did not arrive. Both are
            // "site connected" without a usable global IP.
            self.manager_state = manager_state::CONNECTED_SITE;
            self.connectivity = nm_common::consts::connectivity::LIMITED;
            self.primary_connection = None;
        } else {
            self.manager_state = manager_state::DISCONNECTED;
            self.connectivity = nm_common::consts::connectivity::UNKNOWN;
            self.primary_connection = None;
        }
    }

    pub fn forget_active_for_device(&mut self, device_path: &OwnedObjectPath) {
        let removed: Vec<_> = self
            .active
            .iter()
            .filter(|(_, a)| &a.device_path == device_path)
            .map(|(p, _)| p.clone())
            .collect();
        for path in removed {
            self.active.remove(&path);
        }
    }

    pub fn latest_scan_time(&self) -> i64 {
        self.devices
            .values()
            .map(|d| d.last_scan)
            .max()
            .unwrap_or(0)
    }
}
