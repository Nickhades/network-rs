//! Numeric constants from the NetworkManager D-Bus API.
//!
//! Values are copied from NetworkManager's libnm enums; frontends compare these
//! numbers, so they must not be changed.

pub mod manager_state {
    pub const UNKNOWN: u32 = 0;
    pub const ASLEEP: u32 = 10;
    pub const DISCONNECTED: u32 = 20;
    pub const CONNECTING: u32 = 30;
    pub const CONNECTED_LOCAL: u32 = 40;
    pub const CONNECTED_SITE: u32 = 50;
    pub const CONNECTED_GLOBAL: u32 = 60;
}

pub mod connectivity {
    pub const UNKNOWN: u32 = 0;
    pub const NONE: u32 = 1;
    pub const PORTAL: u32 = 2;
    pub const LIMITED: u32 = 3;
    pub const FULL: u32 = 4;
}

pub mod device_state {
    pub const UNKNOWN: u32 = 0;
    pub const UNMANAGED: u32 = 10;
    pub const UNAVAILABLE: u32 = 20;
    pub const DISCONNECTED: u32 = 30;
    pub const PREPARE: u32 = 40;
    pub const CONFIG: u32 = 50;
    pub const NEED_AUTH: u32 = 60;
    pub const IP_CONFIG: u32 = 70;
    pub const IP_CHECK: u32 = 80;
    pub const SECONDARIES: u32 = 90;
    pub const ACTIVATED: u32 = 100;
    pub const DEACTIVATING: u32 = 110;
    pub const FAILED: u32 = 120;
}

pub mod device_state_reason {
    pub const NONE: u32 = 0;
    pub const UNKNOWN: u32 = 1;
    pub const NOW_MANAGED: u32 = 2;
    pub const CONFIG_FAILED: u32 = 4;
    pub const SUPPLICANT_DISCONNECT: u32 = 8;
    pub const SUPPLICANT_FAILED: u32 = 10;
    pub const SUPPLICANT_TIMEOUT: u32 = 11;
    pub const NO_SECRETS: u32 = 7;
    pub const USER: u32 = 39;
}

pub mod device_type {
    pub const UNKNOWN: u32 = 0;
    pub const ETHERNET: u32 = 1;
    pub const WIFI: u32 = 2;
    pub const UNUSED1: u32 = 3;
    pub const UNUSED2: u32 = 4;
    pub const BT: u32 = 5;
    pub const OLPC_MESH: u32 = 6;
    pub const WIMAX: u32 = 7;
    pub const MODEM: u32 = 8;
    pub const INFINIBAND: u32 = 9;
    pub const BOND: u32 = 10;
    pub const VLAN: u32 = 11;
    pub const ADSL: u32 = 12;
    pub const BRIDGE: u32 = 13;
    pub const GENERIC: u32 = 14;
    pub const TEAM: u32 = 15;
    pub const TUN: u32 = 16;
    pub const IP_TUNNEL: u32 = 17;
    pub const MACVLAN: u32 = 18;
    pub const VXLAN: u32 = 19;
    pub const VETH: u32 = 20;
    pub const WIREGUARD: u32 = 29;
    pub const LOOPBACK: u32 = 32;
}

pub mod device_capability {
    pub const NONE: u32 = 0;
    pub const NM_SUPPORTED: u32 = 1;
    pub const CARRIER_DETECT: u32 = 2;
    pub const IS_SOFTWARE: u32 = 4;
}

pub mod active_connection_state {
    pub const UNKNOWN: u32 = 0;
    pub const ACTIVATING: u32 = 1;
    pub const ACTIVATED: u32 = 2;
    pub const DEACTIVATING: u32 = 3;
    pub const DEACTIVATED: u32 = 4;
}

pub mod wireless_mode {
    pub const UNKNOWN: u32 = 0;
    pub const ADHOC: u32 = 1;
    pub const INFRA: u32 = 2;
    pub const AP: u32 = 3;
    pub const MESH: u32 = 4;
}

pub mod ap_flags {
    pub const NONE: u32 = 0;
    pub const PRIVACY: u32 = 1;
}

pub mod ap_security_flags {
    pub const NONE: u32 = 0;
    pub const PAIR_WEP40: u32 = 0x1;
    pub const PAIR_WEP104: u32 = 0x2;
    pub const PAIR_TKIP: u32 = 0x4;
    pub const PAIR_CCMP: u32 = 0x8;
    pub const GROUP_WEP40: u32 = 0x10;
    pub const GROUP_WEP104: u32 = 0x20;
    pub const GROUP_TKIP: u32 = 0x40;
    pub const GROUP_CCMP: u32 = 0x80;
    pub const KEY_MGMT_PSK: u32 = 0x100;
    pub const KEY_MGMT_802_1X: u32 = 0x200;
    pub const KEY_MGMT_SAE: u32 = 0x400;
    pub const KEY_MGMT_OWE: u32 = 0x800;
    pub const KEY_MGMT_OWE_TRANSITION: u32 = 0x1000;
    pub const KEY_MGMT_SAE_EXT: u32 = 0x2000;
}

pub mod wifi_device_capability {
    pub const NONE: u32 = 0;
    pub const CIPHER_WEP40: u32 = 0x1;
    pub const CIPHER_WEP104: u32 = 0x2;
    pub const CIPHER_TKIP: u32 = 0x4;
    pub const CIPHER_CCMP: u32 = 0x8;
    pub const WPA: u32 = 0x10;
    pub const RSN: u32 = 0x20;
}

pub mod nm80211_mode {
    pub const UNKNOWN: u32 = 0;
    pub const ADHOC: u32 = 1;
    pub const INFRA: u32 = 2;
    pub const AP: u32 = 3;
    pub const MESH: u32 = 4;
}

pub mod metered {
    pub const UNKNOWN: u32 = 0;
    pub const YES: u32 = 1;
    pub const NO: u32 = 2;
    pub const GUESS_YES: u32 = 3;
    pub const GUESS_NO: u32 = 4;
}
