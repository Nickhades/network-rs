//! Connection profile persistence and NetworkManager settings dictionaries.
//!
//! Profiles live as JSON in the state directory. They are deliberately kept
//! in a small, human-readable format instead of serializing arbitrary
//! zvariant values: only the fields this daemon can actually apply are stored.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nm_common::ssid::ssid_to_string;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zbus::{OwnedVariant, Variant};

use crate::model::{ConnectionProfile, ProfileKind};

pub type SettingsMap = HashMap<String, HashMap<String, OwnedVariant>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredProfile {
    pub uuid: String,
    pub id: String,
    /// "wifi" or "ethernet".
    pub kind: String,
    #[serde(default)]
    pub ifname: Option<String>,
    #[serde(default)]
    pub ssid: Vec<u8>,
    #[serde(default)]
    pub psk: Option<String>,
    #[serde(default)]
    pub key_mgmt: Option<String>,
    #[serde(default = "default_true")]
    pub autoconnect: bool,
    #[serde(default)]
    pub timestamp: u64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct ProfileDraft {
    pub uuid: String,
    pub id: String,
    pub kind: ProfileKind,
    pub ifname: Option<String>,
    pub ssid: Vec<u8>,
    pub psk: Option<String>,
    pub key_mgmt: Option<String>,
    pub autoconnect: bool,
    pub timestamp: u64,
}

pub fn owned<T>(value: T) -> OwnedVariant
where
    Variant<'static>: From<T>,
{
    OwnedVariant::try_from(Variant::from(value)).expect("owned variant")
}

pub fn get_string(map: &HashMap<String, OwnedVariant>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| String::try_from(value).ok())
}

pub fn get_bytes(map: &HashMap<String, OwnedVariant>, key: &str) -> Option<Vec<u8>> {
    map.get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| Vec::<u8>::try_from(value).ok())
}

pub fn get_bool(map: &HashMap<String, OwnedVariant>, key: &str) -> Option<bool> {
    map.get(key).and_then(|value| bool::try_from(value).ok())
}

pub fn get_u64(map: &HashMap<String, OwnedVariant>, key: &str) -> Option<u64> {
    map.get(key).and_then(|value| u64::try_from(value).ok())
}

fn wifi_key_mgmt_to_wpa(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "wpa-psk" | "wpa2-psk" | "psk" => "WPA-PSK".to_string(),
        "sae" | "wpa3-sae" => "SAE".to_string(),
        "wpa-eap" | "802.1x" | "eap" => "WPA-EAP".to_string(),
        "owe" => "OWE".to_string(),
        "wep" => "WEP".to_string(),
        "none" | "open" | "" => "NONE".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

pub fn wpa_key_mgmt_to_wifi(value: &str) -> &'static str {
    match value.to_ascii_uppercase().as_str() {
        "WPA-PSK" => "wpa-psk",
        "SAE" => "sae",
        "WPA-EAP" => "wpa-eap",
        "OWE" => "owe",
        "WEP" => "wep",
        _ => "none",
    }
}

/// Translate an a{sa{sv}} settings dictionary into the fields we can apply.
pub fn parse_settings(
    settings: &SettingsMap,
    existing: Option<&ConnectionProfile>,
) -> Result<ProfileDraft, String> {
    let connection = settings
        .get("connection")
        .ok_or_else(|| "missing connection setting".to_string())?;
    let type_ = get_string(connection, "type")
        .or_else(|| existing.map(|p| p.kind.connection_type().to_string()))
        .ok_or_else(|| "missing connection.type".to_string())?;

    let kind = match type_.as_str() {
        "802-11-wireless" => ProfileKind::Wifi,
        "802-3-ethernet" => ProfileKind::Ethernet,
        other => return Err(format!("unsupported connection type {other}")),
    };

    let mut uuid = existing
        .map(|p| p.uuid.clone())
        .or_else(|| get_string(connection, "uuid").filter(|s| !s.is_empty()))
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    if Uuid::parse_str(&uuid).is_err() {
        // The UUID becomes a filename. Never accept path components or an
        // arbitrary string from a D-Bus caller.
        uuid = Uuid::new_v4().to_string();
    }

    let mut ssid = Vec::new();
    let mut psk = existing.and_then(|p| p.psk.clone());
    let mut key_mgmt = existing.and_then(|p| p.key_mgmt.clone());
    let mut ifname = existing.and_then(|p| p.ifname.clone());

    match kind {
        ProfileKind::Wifi => {
            let wireless = settings
                .get("802-11-wireless")
                .ok_or_else(|| "missing 802-11-wireless setting".to_string())?;
            ssid = get_bytes(wireless, "ssid")
                .or_else(|| existing.map(|p| p.ssid.clone()))
                .ok_or_else(|| "missing 802-11-wireless.ssid".to_string())?;
            if ssid.is_empty() {
                return Err("SSID must not be empty".to_string());
            }
            if ssid.len() > 32 {
                return Err("SSID is longer than 32 bytes".to_string());
            }
            if let Some(security) = settings.get("802-11-wireless-security") {
                if let Some(value) = get_string(security, "key-mgmt") {
                    key_mgmt = Some(wifi_key_mgmt_to_wpa(&value));
                }
                if let Some(value) = get_string(security, "psk").filter(|s| !s.is_empty()) {
                    psk = Some(value);
                }
            }
            if key_mgmt.is_none() {
                key_mgmt = Some("NONE".to_string());
            }
        }
        ProfileKind::Ethernet => {
            ssid.clear();
            psk = None;
            key_mgmt = None;
        }
    }

    let id = get_string(connection, "id")
        .filter(|s| !s.is_empty())
        .or_else(|| existing.map(|p| p.id.clone()))
        .unwrap_or_else(|| {
            if kind == ProfileKind::Wifi {
                ssid_to_string(&ssid)
            } else {
                ifname
                    .clone()
                    .unwrap_or_else(|| "Wired connection".to_string())
            }
        });

    let autoconnect = get_bool(connection, "autoconnect")
        .or_else(|| existing.map(|p| p.autoconnect))
        .unwrap_or(true);
    let timestamp = get_u64(connection, "timestamp")
        .or_else(|| existing.map(|p| p.timestamp))
        .unwrap_or_else(now_unix);

    if kind == ProfileKind::Wifi && ifname.is_none() {
        ifname = None;
    }

    Ok(ProfileDraft {
        uuid,
        id,
        kind,
        ifname,
        ssid,
        psk,
        key_mgmt,
        autoconnect,
        timestamp,
    })
}

pub fn profile_to_settings(profile: &ConnectionProfile, include_secrets: bool) -> SettingsMap {
    let mut settings = SettingsMap::new();

    let mut connection = HashMap::new();
    connection.insert("id".to_string(), owned(profile.id.clone()));
    connection.insert("uuid".to_string(), owned(profile.uuid.clone()));
    connection.insert(
        "type".to_string(),
        owned(profile.kind.connection_type().to_string()),
    );
    connection.insert("autoconnect".to_string(), owned(profile.autoconnect));
    connection.insert("timestamp".to_string(), owned(profile.timestamp));
    settings.insert("connection".to_string(), connection);

    match profile.kind {
        ProfileKind::Wifi => {
            let mut wireless = HashMap::new();
            wireless.insert("ssid".to_string(), owned(profile.ssid.clone()));
            wireless.insert("mode".to_string(), owned("infrastructure".to_string()));
            if profile.psk.is_some() {
                wireless.insert(
                    "security".to_string(),
                    owned("802-11-wireless-security".to_string()),
                );
            }
            settings.insert("802-11-wireless".to_string(), wireless);

            let key_mgmt = profile.key_mgmt.as_deref().unwrap_or("NONE");
            if key_mgmt != "NONE" {
                let mut security = HashMap::new();
                security.insert(
                    "key-mgmt".to_string(),
                    owned(wpa_key_mgmt_to_wifi(key_mgmt).to_string()),
                );
                if key_mgmt == "WPA-PSK" {
                    security.insert("auth-alg".to_string(), owned("open".to_string()));
                }
                if include_secrets && let Some(psk) = &profile.psk {
                    security.insert("psk".to_string(), owned(psk.clone()));
                }
                settings.insert("802-11-wireless-security".to_string(), security);
            }
        }
        ProfileKind::Ethernet => {
            let mut wired = HashMap::new();
            wired.insert("auto-negotiate".to_string(), owned(true));
            settings.insert("802-3-ethernet".to_string(), wired);
        }
    }

    let mut ipv4 = HashMap::new();
    ipv4.insert("method".to_string(), owned("auto".to_string()));
    ipv4.insert("may-fail".to_string(), owned(true));
    settings.insert("ipv4".to_string(), ipv4);

    let mut ipv6 = HashMap::new();
    ipv6.insert("method".to_string(), owned("auto".to_string()));
    ipv6.insert("may-fail".to_string(), owned(true));
    settings.insert("ipv6".to_string(), ipv6);

    settings
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn connections_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("connections")
}

pub fn load_profiles(state_dir: &Path) -> Vec<StoredProfile> {
    let dir = connections_dir(state_dir);
    let mut profiles = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return profiles;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match fs::read_to_string(&path)
            .and_then(|text| serde_json::from_str::<StoredProfile>(&text).map_err(io::Error::from))
        {
            Ok(profile) => {
                if Uuid::parse_str(&profile.uuid).is_err() {
                    tracing::warn!(path = %path.display(), "ignoring profile with invalid uuid");
                    continue;
                }
                profiles.push(profile);
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "ignoring unreadable connection profile")
            }
        }
    }
    profiles.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    profiles
}

pub fn save_profile(state_dir: &Path, profile: &StoredProfile) -> io::Result<()> {
    if Uuid::parse_str(&profile.uuid).is_err() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "profile uuid is not a UUID",
        ));
    }
    let dir = connections_dir(state_dir);
    fs::create_dir_all(&dir)?;
    set_mode(&dir, 0o700)?;
    let path = dir.join(format!("{}.json", profile.uuid));
    let tmp = dir.join(format!(".{}.json.tmp", profile.uuid));
    let text = serde_json::to_string_pretty(profile)?;
    fs::write(&tmp, text)?;
    set_mode(&tmp, 0o600)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn delete_profile(state_dir: &Path, uuid: &str) -> io::Result<()> {
    let path = connections_dir(state_dir).join(format!("{uuid}.json"));
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

impl StoredProfile {
    pub fn from_draft(draft: &ProfileDraft) -> Self {
        let kind = match draft.kind {
            ProfileKind::Wifi => "wifi",
            ProfileKind::Ethernet => "ethernet",
        };
        Self {
            uuid: draft.uuid.clone(),
            id: draft.id.clone(),
            kind: kind.to_string(),
            ifname: draft.ifname.clone(),
            ssid: draft.ssid.clone(),
            psk: draft.psk.clone(),
            key_mgmt: draft.key_mgmt.clone(),
            autoconnect: draft.autoconnect,
            timestamp: draft.timestamp,
        }
    }

    pub fn kind(&self) -> ProfileKind {
        if self.kind == "ethernet" {
            ProfileKind::Ethernet
        } else {
            ProfileKind::Wifi
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ConnectionProfile;

    fn wifi_settings() -> SettingsMap {
        let mut settings = SettingsMap::new();
        let mut connection = HashMap::new();
        connection.insert("id".into(), owned("Cafe".to_string()));
        connection.insert("type".into(), owned("802-11-wireless".to_string()));
        settings.insert("connection".into(), connection);

        let mut wireless = HashMap::new();
        wireless.insert("ssid".into(), owned(b"Cafe WiFi".to_vec()));
        settings.insert("802-11-wireless".into(), wireless);

        let mut security = HashMap::new();
        security.insert("key-mgmt".into(), owned("wpa-psk".to_string()));
        security.insert("psk".into(), owned("hunter2hunter2".to_string()));
        settings.insert("802-11-wireless-security".into(), security);
        settings
    }

    #[test]
    fn parses_wifi_settings() {
        let draft = parse_settings(&wifi_settings(), None).unwrap();
        assert_eq!(draft.kind, ProfileKind::Wifi);
        assert_eq!(draft.ssid, b"Cafe WiFi");
        assert_eq!(draft.key_mgmt.as_deref(), Some("WPA-PSK"));
        assert_eq!(draft.psk.as_deref(), Some("hunter2hunter2"));
        assert_eq!(draft.id, "Cafe");
        assert!(!draft.uuid.is_empty());
    }

    #[test]
    fn preserves_existing_secret_on_update() {
        let profile = ConnectionProfile {
            path: "/x".try_into().unwrap(),
            uuid: "u".into(),
            id: "Cafe".into(),
            kind: ProfileKind::Wifi,
            ifname: None,
            ssid: b"Cafe WiFi".to_vec(),
            psk: Some("oldsecret".into()),
            key_mgmt: Some("WPA-PSK".into()),
            autoconnect: true,
            timestamp: 1,
        };
        let mut settings = wifi_settings();
        settings
            .get_mut("802-11-wireless-security")
            .unwrap()
            .remove("psk");
        let draft = parse_settings(&settings, Some(&profile)).unwrap();
        assert_eq!(draft.psk.as_deref(), Some("oldsecret"));
    }

    #[test]
    fn roundtrips_settings() {
        let draft = parse_settings(&wifi_settings(), None).unwrap();
        let profile = ConnectionProfile {
            path: "/x".try_into().unwrap(),
            uuid: draft.uuid.clone(),
            id: draft.id.clone(),
            kind: draft.kind,
            ifname: None,
            ssid: draft.ssid.clone(),
            psk: draft.psk.clone(),
            key_mgmt: draft.key_mgmt.clone(),
            autoconnect: true,
            timestamp: 1,
        };
        let settings = profile_to_settings(&profile, true);
        assert_eq!(
            get_string(&settings["connection"], "id").as_deref(),
            Some("Cafe")
        );
        assert_eq!(
            get_bytes(&settings["802-11-wireless"], "ssid"),
            Some(b"Cafe WiFi".to_vec())
        );
        assert_eq!(
            get_string(&settings["802-11-wireless-security"], "psk").as_deref(),
            Some("hunter2hunter2")
        );
        let draft2 = parse_settings(&settings, None).unwrap();
        assert_eq!(draft2.uuid, profile.uuid);
    }

    #[test]
    fn rejects_oversized_ssid() {
        let mut settings = wifi_settings();
        settings
            .get_mut("802-11-wireless")
            .unwrap()
            .insert("ssid".into(), owned(vec![b'x'; 33]));
        assert!(parse_settings(&settings, None).is_err());
    }

    #[test]
    fn rejects_path_traversal_uuid() {
        let mut settings = wifi_settings();
        settings
            .get_mut("connection")
            .unwrap()
            .insert("uuid".into(), owned("../../etc/passwd".to_string()));
        let draft = parse_settings(&settings, None).unwrap();
        assert!(Uuid::parse_str(&draft.uuid).is_ok());
        assert!(!draft.uuid.contains('/'));
    }

    #[test]
    fn save_refuses_invalid_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let stored = StoredProfile {
            uuid: "../escape".into(),
            id: "x".into(),
            kind: "wifi".into(),
            ifname: None,
            ssid: b"x".to_vec(),
            psk: None,
            key_mgmt: Some("NONE".into()),
            autoconnect: true,
            timestamp: 0,
        };
        assert!(save_profile(dir.path(), &stored).is_err());
    }
    #[test]
    fn persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let draft = parse_settings(&wifi_settings(), None).unwrap();
        let stored = StoredProfile::from_draft(&draft);
        save_profile(dir.path(), &stored).unwrap();
        let loaded = load_profiles(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].uuid, stored.uuid);
        assert_eq!(loaded[0].psk.as_deref(), Some("hunter2hunter2"));
    }
}
