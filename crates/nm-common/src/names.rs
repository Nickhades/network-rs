//! Bus name, object paths and interface names used by NetworkManager.

/// The well-known bus name the daemon owns.
pub const NM_BUS_NAME: &str = "org.freedesktop.NetworkManager";
pub const NM_BUS_NAME_PREFIX: &str = "org.freedesktop.NetworkManager";

pub const NM_PATH: &str = "/org/freedesktop/NetworkManager";
pub const NM_DEVICES_PATH: &str = "/org/freedesktop/NetworkManager/Devices";
pub const NM_ACCESS_POINTS_PATH: &str = "/org/freedesktop/NetworkManager/AccessPoints";
pub const NM_SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
pub const NM_ACTIVE_CONNECTIONS_PATH: &str = "/org/freedesktop/NetworkManager/ActiveConnection";
pub const NM_IP4_CONFIG_PATH: &str = "/org/freedesktop/NetworkManager/IP4Config";
pub const NM_IP6_CONFIG_PATH: &str = "/org/freedesktop/NetworkManager/IP6Config";
pub const NM_DHCP4_CONFIG_PATH: &str = "/org/freedesktop/NetworkManager/DHCP4Config";
pub const NM_DHCP6_CONFIG_PATH: &str = "/org/freedesktop/NetworkManager/DHCP6Config";
pub const NM_AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
pub const NM_DNS_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager/DnsManager";

pub mod iface {
    pub const MANAGER: &str = "org.freedesktop.NetworkManager";
    pub const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
    pub const DEVICE_WIRED: &str = "org.freedesktop.NetworkManager.Device.Wired";
    pub const DEVICE_WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
    pub const DEVICE_STATISTICS: &str = "org.freedesktop.NetworkManager.Device.Statistics";
    pub const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
    pub const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
    pub const SETTINGS_CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
    pub const ACTIVE_CONNECTION: &str = "org.freedesktop.NetworkManager.Connection.Active";
    pub const IP4_CONFIG: &str = "org.freedesktop.NetworkManager.IP4Config";
    pub const IP6_CONFIG: &str = "org.freedesktop.NetworkManager.IP6Config";
    pub const DHCP4_CONFIG: &str = "org.freedesktop.NetworkManager.DHCP4Config";
    pub const DHCP6_CONFIG: &str = "org.freedesktop.NetworkManager.DHCP6Config";
    pub const SECRET_AGENT: &str = "org.freedesktop.NetworkManager.SecretAgent";
    pub const DNS_MANAGER: &str = "org.freedesktop.NetworkManager.DnsManager";
    pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
    pub const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
}

/// The wpa_supplicant D-Bus service (only present when wpa_supplicant was
/// built with CONFIG_CTRL_IFACE_DBUS, which stock FreeBSD is not).
pub const WPA_SUPPLICANT_BUS_NAME: &str = "fi.w1.wpa_supplicant1";
pub const WPA_SUPPLICANT_PATH: &str = "/fi/w1/wpa_supplicant1";
pub const WPA_SUPPLICANT_IFACE: &str = "fi.w1.wpa_supplicant1";
pub const WPA_SUPPLICANT_IFACE_INTERFACE: &str = "fi.w1.wpa_supplicant1.Interface";
pub const WPA_SUPPLICANT_BSS_INTERFACE: &str = "fi.w1.wpa_supplicant1.BSS";
