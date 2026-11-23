//! The daemon core: state, D-Bus object lifecycle, and activation logic.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nm_common::consts::{active_connection_state, ap_flags, device_state, device_state_reason};
use nm_common::names;
use nm_common::ssid::ssid_to_string;
use std::sync::OnceLock;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock, mpsc};
use uuid::Uuid;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, ObjectServer, OwnedObjectPath, OwnedVariant};

use crate::backend::{Backend, BackendEvent, ConnectRequest, WifiStatus};
use crate::config::Config;
use crate::dbus::{
    AccessPointIface, ActiveConnectionIface, Bus, DeviceIface, Ip4ConfigIface, ManagerIface,
    SettingsConnectionIface, SettingsIface, WiredIface, WirelessIface, root_path,
};
use crate::model::{
    ActiveEntry, ApEntry, ConnectionProfile, CoreState, DeviceEntry, DeviceKind, Ip4Info,
    ProfileKind,
};
use crate::settings::{self as profile_store, ProfileDraft, SettingsMap, StoredProfile, owned};

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unknown object: {0}")]
    UnknownObject(String),
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("not supported: {0}")]
    NotSupported(String),
    #[error("backend error: {0}")]
    Backend(#[from] crate::backend::BackendError),
    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct Core {
    pub config: Config,
    pub state: RwLock<CoreState>,
    pub backend: Arc<dyn Backend>,
    bus: OnceLock<Bus>,
    refresh_lock: Mutex<()>,
}

impl Core {
    pub fn new(config: Config, backend: Arc<dyn Backend>) -> Arc<Self> {
        let mut state = CoreState::new(crate::system::hostname());
        for stored in profile_store::load_profiles(&config.state_dir) {
            if state.connections.values().any(|p| p.uuid == stored.uuid) {
                continue;
            }
            let path = state.next_connection_path();
            let profile = profile_from_stored(&stored, path.clone());
            state.connections.insert(path, profile);
        }
        Arc::new(Self {
            config,
            state: RwLock::new(state),
            backend,
            bus: OnceLock::new(),
            refresh_lock: Mutex::new(()),
        })
    }

    pub fn set_bus(&self, connection: Connection, server: ObjectServer) {
        let _ = self.bus.set(Bus { connection, server });
    }

    pub fn bus(&self) -> Option<&Bus> {
        self.bus.get()
    }

    pub async fn start(self: &Arc<Self>) -> Result<(), CoreError> {
        let bus = self
            .bus
            .get()
            .ok_or_else(|| CoreError::NotSupported("bus is not connected".to_string()))?;
        bus.server
            .at(names::NM_PATH, ManagerIface::new(self.clone()))
            .await?;
        bus.server
            .at(names::NM_SETTINGS_PATH, SettingsIface::new(self.clone()))
            .await?;

        let profiles: Vec<ConnectionProfile> = self
            .state
            .read()
            .await
            .connections
            .values()
            .cloned()
            .collect();
        for profile in &profiles {
            self.register_settings_connection(profile).await?;
        }

        self.load_devices().await?;
        self.start_backend_events();
        self.start_netlink();
        self.spawn_initial_sync();
        Ok(())
    }

    // ------------------------------------------------------------------
    // Device lifecycle
    // ------------------------------------------------------------------

    async fn load_devices(self: &Arc<Self>) -> Result<(), CoreError> {
        let dumped = tokio::task::spawn_blocking(crate::netlink::dump_links).await;
        let links = match dumped {
            Ok(Ok(links)) if !links.is_empty() => links,
            Ok(Ok(_)) => {
                tracing::warn!("netlink returned no links; falling back to getifaddrs");
                fallback_links()
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "netlink link dump failed; falling back to getifaddrs");
                fallback_links()
            }
            Err(error) => {
                tracing::warn!(%error, "netlink dump task failed; falling back to getifaddrs");
                fallback_links()
            }
        };
        let mut links = links;
        links.sort_by_key(|link| link.ifindex);
        for link in links {
            if let Err(error) = self.upsert_device(link).await {
                tracing::warn!(%error, "failed to expose interface");
            }
        }
        Ok(())
    }

    fn classify(&self, ifname: &str) -> Option<DeviceKind> {
        if self
            .config
            .wifi_interfaces
            .iter()
            .any(|name| name == ifname)
            || ifname.starts_with("wlan")
        {
            return Some(DeviceKind::Wifi);
        }
        if !self.config.include_wired {
            return None;
        }
        if is_ignored_interface(ifname) {
            return None;
        }
        Some(DeviceKind::Ethernet)
    }

    async fn upsert_device(
        self: &Arc<Self>,
        link: crate::netlink::LinkInfo,
    ) -> Result<(), CoreError> {
        let Some(kind) = self.classify(&link.ifname) else {
            return Ok(());
        };
        let ip4 = self.ip4_for(&link.ifname);
        let mut changed_state: Option<(u32, u32, u32)> = None;
        let mut created = false;
        let path;

        {
            let mut state = self.state.write().await;
            if let Some(device) = state.devices.get_mut(&link.ifname) {
                device.mac = link.mac.clone();
                device.mtu = link.mtu;
                device.ip4 = ip4.clone();
                path = device.path.clone();
                if device.active_connection.is_none() && kind == DeviceKind::Ethernet {
                    let next = if !link.is_up() {
                        device_state::UNAVAILABLE
                    } else if ip4.is_some() {
                        device_state::ACTIVATED
                    } else if device.state == device_state::UNAVAILABLE
                        || device.state == device_state::ACTIVATED
                    {
                        device_state::DISCONNECTED
                    } else {
                        device.state
                    };
                    if next != device.state {
                        let old = device.state;
                        device.state = next;
                        device.state_reason = device_state_reason::NONE;
                        changed_state = Some((old, next, device.state_reason));
                    }
                    state.recompute_manager_state();
                }
            } else {
                let ifindex = if link.ifindex > 0 {
                    link.ifindex
                } else {
                    crate::system::ifindex(&link.ifname).unwrap_or(0)
                };
                let mut device = DeviceEntry::new(
                    link.ifname.clone(),
                    ifindex,
                    kind,
                    link.mac.clone(),
                    link.mtu,
                );
                if kind == DeviceKind::Ethernet {
                    device.state = if !link.is_up() {
                        device_state::UNAVAILABLE
                    } else if ip4.is_some() {
                        device_state::ACTIVATED
                    } else {
                        device_state::DISCONNECTED
                    };
                }
                device.ip4 = ip4.clone();
                path = device.path.clone();
                state.devices.insert(link.ifname.clone(), device);
                state.recompute_manager_state();
                created = true;
            }
        }

        if created {
            self.register_device(&link.ifname).await?;
            if ip4.is_some() {
                self.ensure_ip4_config(&link.ifname).await;
            }
            self.notify_manager().await;
            self.notify_device_added(&path).await;
            if ip4.is_some() {
                self.notify_device(&link.ifname, None).await;
            }
        } else if let Some(transition) = changed_state {
            if ip4.is_some() {
                self.ensure_ip4_config(&link.ifname).await;
            }
            self.notify_device(&link.ifname, Some(transition)).await;
            self.notify_manager().await;
        }
        Ok(())
    }

    async fn register_device(self: &Arc<Self>, ifname: &str) -> Result<(), CoreError> {
        let (path, kind) = {
            let state = self.state.read().await;
            let device = state
                .devices
                .get(ifname)
                .ok_or_else(|| CoreError::UnknownObject(ifname.to_string()))?;
            (device.path.clone(), device.kind)
        };
        let bus = self
            .bus
            .get()
            .ok_or_else(|| CoreError::NotSupported("bus is not connected".to_string()))?;
        if bus.server.interface::<_, DeviceIface>(&path).await.is_ok() {
            return Ok(());
        }
        bus.server
            .at(
                path.clone(),
                DeviceIface::new(self.clone(), ifname.to_string()),
            )
            .await?;
        match kind {
            DeviceKind::Wifi => {
                bus.server
                    .at(
                        path.clone(),
                        WirelessIface::new(self.clone(), ifname.to_string()),
                    )
                    .await?;
            }
            DeviceKind::Ethernet => {
                bus.server
                    .at(
                        path.clone(),
                        WiredIface::new(self.clone(), ifname.to_string()),
                    )
                    .await?;
            }
            DeviceKind::Other => {}
        }
        Ok(())
    }

    async fn remove_device(self: &Arc<Self>, ifname: &str) {
        let (path, ap_paths, active_paths) = {
            let mut state = self.state.write().await;
            let Some(device) = state.devices.remove(ifname) else {
                return;
            };
            let ap_keys: Vec<String> = state
                .aps
                .iter()
                .filter(|(_, ap)| ap.ifname == ifname)
                .map(|(key, _)| key.clone())
                .collect();
            let ap_paths: Vec<OwnedObjectPath> = ap_keys
                .iter()
                .filter_map(|key| state.aps.remove(key).map(|ap| ap.path))
                .collect();
            let active_paths: Vec<OwnedObjectPath> = state
                .active
                .iter()
                .filter(|(_, active)| active.device_path == device.path)
                .map(|(path, _)| path.clone())
                .collect();
            for path in &active_paths {
                state.active.remove(path);
            }
            state.recompute_manager_state();
            (device.path, ap_paths, active_paths)
        };

        if let Some(bus) = self.bus.get() {
            for ap in &ap_paths {
                let _ = bus.server.remove::<AccessPointIface, _>(ap).await;
            }
            for active in &active_paths {
                let _ = bus.server.remove::<ActiveConnectionIface, _>(active).await;
            }
            let _ = bus.server.remove::<WirelessIface, _>(&path).await;
            let _ = bus.server.remove::<WiredIface, _>(&path).await;
            let _ = bus.server.remove::<DeviceIface, _>(&path).await;
        }
        self.notify_manager().await;
        self.notify_device_removed(&path).await;
    }

    // ------------------------------------------------------------------
    // Access points
    // ------------------------------------------------------------------

    pub async fn refresh_access_points(self: &Arc<Self>, ifname: &str) -> Result<(), CoreError> {
        let _guard = self.refresh_lock.lock().await;
        let aps = self.backend.scan_results(ifname).await?;
        let now = unix_now() as i64;
        let mut created: Vec<(String, OwnedObjectPath)> = Vec::new();
        let mut removed: Vec<OwnedObjectPath> = Vec::new();
        let mut updated: Vec<OwnedObjectPath> = Vec::new();

        {
            let mut state = self.state.write().await;
            if !state.devices.contains_key(ifname) {
                return Ok(());
            }
            let existing: HashSet<String> = state
                .aps
                .iter()
                .filter(|(_, ap)| ap.ifname == ifname)
                .map(|(key, _)| key.clone())
                .collect();
            let mut seen = HashSet::new();

            for scan_ap in &aps {
                let key = ApEntry::ap_key(ifname, &scan_ap.bssid);
                seen.insert(key.clone());
                let flags = if scan_ap.security.privacy {
                    ap_flags::PRIVACY
                } else {
                    0
                };
                if let Some(existing) = state.aps.get_mut(&key) {
                    existing.ssid = scan_ap.ssid.clone();
                    existing.frequency = scan_ap.frequency;
                    existing.strength = scan_ap.strength();
                    existing.flags = flags;
                    existing.wpa_flags = scan_ap.security.wpa_flags;
                    existing.rsn_flags = scan_ap.security.rsn_flags;
                    existing.security = scan_ap.security;
                    existing.last_seen = 0;
                    updated.push(existing.path.clone());
                } else {
                    let path = state.next_ap_path();
                    state.aps.insert(
                        key.clone(),
                        ApEntry {
                            ifname: ifname.to_string(),
                            bssid: scan_ap.bssid.clone(),
                            ssid: scan_ap.ssid.clone(),
                            frequency: scan_ap.frequency,
                            strength: scan_ap.strength(),
                            flags,
                            wpa_flags: scan_ap.security.wpa_flags,
                            rsn_flags: scan_ap.security.rsn_flags,
                            last_seen: 0,
                            path: path.clone(),
                            security: scan_ap.security,
                        },
                    );
                    created.push((scan_ap.bssid.clone(), path));
                }
            }

            if !aps.is_empty() {
                for key in existing.difference(&seen) {
                    if let Some(ap) = state.aps.remove(key) {
                        removed.push(ap.path);
                    }
                }
            }
            if let Some(device) = state.devices.get_mut(ifname) {
                device.last_scan = now;
            }
        }

        for (bssid, path) in &created {
            if let Err(error) = self.register_access_point(ifname, bssid, path).await {
                tracing::warn!(%error, ifname, bssid, "failed to register access point");
            }
        }
        if let Some(bus) = self.bus.get() {
            for path in &removed {
                let _ = bus.server.remove::<AccessPointIface, _>(path).await;
            }
        }
        for path in &updated {
            self.notify_access_point(path).await;
        }
        self.notify_wireless(
            ifname,
            &created.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>(),
            &removed,
        )
        .await;
        Ok(())
    }

    async fn register_access_point(
        self: &Arc<Self>,
        ifname: &str,
        bssid: &str,
        path: &OwnedObjectPath,
    ) -> Result<(), CoreError> {
        let bus = self
            .bus
            .get()
            .ok_or_else(|| CoreError::NotSupported("bus is not connected".to_string()))?;
        if bus
            .server
            .interface::<_, AccessPointIface>(path)
            .await
            .is_ok()
        {
            return Ok(());
        }
        bus.server
            .at(
                path.clone(),
                AccessPointIface::new(self.clone(), ifname.to_string(), bssid.to_string()),
            )
            .await?;
        Ok(())
    }

    pub async fn request_scan(self: &Arc<Self>, ifname: &str) -> Result<(), CoreError> {
        self.backend.scan(ifname).await?;
        let core = self.clone();
        let ifname = ifname.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Err(error) = core.refresh_access_points(&ifname).await {
                tracing::debug!(%error, ifname, "scan result refresh failed");
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let _ = core.refresh_access_points(&ifname).await;
        });
        Ok(())
    }

    // ------------------------------------------------------------------
    // Settings
    // ------------------------------------------------------------------

    pub async fn connection_settings(
        &self,
        path: &OwnedObjectPath,
        include_secrets: bool,
    ) -> Result<SettingsMap, CoreError> {
        let profile = self
            .state
            .read()
            .await
            .connections
            .get(path)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(path.to_string()))?;
        Ok(profile_store::profile_to_settings(
            &profile,
            include_secrets,
        ))
    }

    pub async fn connection_settings_by_uuid(
        &self,
        uuid: &str,
        include_secrets: bool,
    ) -> Result<SettingsMap, CoreError> {
        let profile = self
            .state
            .read()
            .await
            .connections
            .values()
            .find(|profile| profile.uuid == uuid)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(uuid.to_string()))?;
        Ok(profile_store::profile_to_settings(
            &profile,
            include_secrets,
        ))
    }

    pub async fn connection_secrets_by_uuid(&self, uuid: &str) -> Result<SettingsMap, CoreError> {
        let profile = self
            .state
            .read()
            .await
            .connections
            .values()
            .find(|profile| profile.uuid == uuid)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(uuid.to_string()))?;
        let mut settings = SettingsMap::new();
        if let Some(psk) = profile.psk {
            let mut security = HashMap::new();
            security.insert("psk".to_string(), profile_store::owned(psk));
            settings.insert("802-11-wireless-security".to_string(), security);
        }
        Ok(settings)
    }

    pub async fn add_connection_from_settings(
        self: &Arc<Self>,
        settings: SettingsMap,
        persist: bool,
    ) -> Result<OwnedObjectPath, CoreError> {
        let draft =
            profile_store::parse_settings(&settings, None).map_err(CoreError::InvalidArgs)?;
        self.add_connection(draft, persist).await
    }

    pub async fn add_connection(
        self: &Arc<Self>,
        draft: ProfileDraft,
        persist: bool,
    ) -> Result<OwnedObjectPath, CoreError> {
        let uuid = draft.uuid.clone();
        let stored = StoredProfile::from_draft(&draft);
        let profile_fields = (
            draft.id.clone(),
            draft.kind,
            draft.ifname.clone(),
            draft.ssid.clone(),
            draft.psk.clone(),
            draft.key_mgmt.clone(),
            draft.autoconnect,
            draft.timestamp,
        );

        // Reuse an existing profile with the same UUID. Frontends sometimes
        // call AddAndActivateConnection with a profile they already saved;
        // creating a second path for the same UUID would make GetSecrets and
        // the on-disk file ambiguous.
        let existing = {
            self.state
                .read()
                .await
                .connections
                .values()
                .find(|profile| profile.uuid == uuid)
                .map(|profile| profile.path.clone())
        };
        if let Some(path) = existing {
            if persist {
                profile_store::save_profile(&self.config.state_dir, &stored)?;
            }
            let (id, kind, ifname, ssid, psk, key_mgmt, autoconnect, timestamp) = profile_fields;
            self.state.write().await.connections.insert(
                path.clone(),
                ConnectionProfile {
                    path: path.clone(),
                    uuid: uuid.clone(),
                    id,
                    kind,
                    ifname,
                    ssid,
                    psk,
                    key_mgmt,
                    autoconnect,
                    timestamp,
                },
            );
            self.notify_settings_connection_updated(&uuid).await;
            return Ok(path);
        }

        let path = {
            let mut state = self.state.write().await;
            let path = state.next_connection_path();
            let (id, kind, ifname, ssid, psk, key_mgmt, autoconnect, timestamp) = profile_fields;
            state.connections.insert(
                path.clone(),
                ConnectionProfile {
                    path: path.clone(),
                    uuid: uuid.clone(),
                    id,
                    kind,
                    ifname,
                    ssid,
                    psk,
                    key_mgmt,
                    autoconnect,
                    timestamp,
                },
            );
            path
        };

        if persist && let Err(error) = profile_store::save_profile(&self.config.state_dir, &stored)
        {
            self.state.write().await.connections.remove(&path);
            return Err(CoreError::Io(error));
        }

        if let Err(error) = self.register_settings_connection_by_uuid(&uuid).await {
            self.state.write().await.connections.remove(&path);
            if persist {
                let _ = profile_store::delete_profile(&self.config.state_dir, &uuid);
            }
            return Err(error);
        }
        self.notify_settings_new(&path).await;
        Ok(path)
    }

    async fn register_settings_connection(
        self: &Arc<Self>,
        profile: &ConnectionProfile,
    ) -> Result<(), CoreError> {
        self.register_settings_connection_at(&profile.path, &profile.uuid)
            .await
    }

    async fn register_settings_connection_by_uuid(
        self: &Arc<Self>,
        uuid: &str,
    ) -> Result<(), CoreError> {
        let profile = self
            .state
            .read()
            .await
            .connections
            .values()
            .find(|profile| profile.uuid == uuid)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(uuid.to_string()))?;
        self.register_settings_connection(&profile).await
    }

    async fn register_settings_connection_at(
        self: &Arc<Self>,
        path: &OwnedObjectPath,
        uuid: &str,
    ) -> Result<(), CoreError> {
        let bus = self
            .bus
            .get()
            .ok_or_else(|| CoreError::NotSupported("bus is not connected".to_string()))?;
        if bus
            .server
            .interface::<_, SettingsConnectionIface>(path)
            .await
            .is_ok()
        {
            return Ok(());
        }
        bus.server
            .at(
                path.clone(),
                SettingsConnectionIface::new(self.clone(), uuid.to_string()),
            )
            .await?;
        Ok(())
    }

    pub async fn update_connection_settings(
        &self,
        path: &OwnedObjectPath,
        settings: SettingsMap,
        persist: bool,
    ) -> Result<bool, CoreError> {
        let existing = self
            .state
            .read()
            .await
            .connections
            .get(path)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(path.to_string()))?;
        let draft = profile_store::parse_settings(&settings, Some(&existing))
            .map_err(CoreError::InvalidArgs)?;
        let stored = StoredProfile::from_draft(&draft);
        let updated = ConnectionProfile {
            path: path.clone(),
            uuid: existing.uuid.clone(),
            id: draft.id,
            kind: draft.kind,
            ifname: draft.ifname,
            ssid: draft.ssid,
            psk: draft.psk,
            key_mgmt: draft.key_mgmt,
            autoconnect: draft.autoconnect,
            timestamp: draft.timestamp,
        };
        if persist {
            profile_store::save_profile(&self.config.state_dir, &stored)?;
        }
        self.state
            .write()
            .await
            .connections
            .insert(path.clone(), updated);
        self.notify_settings_connection_updated(&existing.uuid)
            .await;
        Ok(true)
    }

    pub async fn update_connection_by_uuid(
        self: &Arc<Self>,
        uuid: &str,
        settings: SettingsMap,
        persist: bool,
    ) -> Result<(), CoreError> {
        let path = self
            .state
            .read()
            .await
            .connections
            .values()
            .find(|profile| profile.uuid == uuid)
            .map(|profile| profile.path.clone())
            .ok_or_else(|| CoreError::UnknownObject(uuid.to_string()))?;
        self.update_connection_settings(&path, settings, persist)
            .await?;
        Ok(())
    }

    pub async fn delete_connection(
        self: &Arc<Self>,
        path: &OwnedObjectPath,
    ) -> Result<(), CoreError> {
        let profile = self
            .state
            .read()
            .await
            .connections
            .get(path)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(path.to_string()))?;
        let dependents: Vec<OwnedObjectPath> = {
            let state = self.state.read().await;
            state
                .active
                .values()
                .filter(|active| &active.connection_path == path)
                .map(|active| active.path.clone())
                .collect()
        };
        for active in dependents {
            let _ = self.deactivate(&active).await;
        }
        self.notify_settings_connection_removed(&profile.uuid).await;
        if let Some(bus) = self.bus.get() {
            let _ = bus.server.remove::<SettingsConnectionIface, _>(path).await;
        }
        self.state.write().await.connections.remove(path);
        if let Err(error) = profile_store::delete_profile(&self.config.state_dir, &profile.uuid) {
            tracing::warn!(%error, uuid = %profile.uuid, "failed to delete profile file");
        }
        self.notify_settings_removed(path).await;
        Ok(())
    }

    pub async fn delete_connection_by_uuid(self: &Arc<Self>, uuid: &str) -> Result<(), CoreError> {
        let path = self
            .state
            .read()
            .await
            .connections
            .values()
            .find(|profile| profile.uuid == uuid)
            .map(|profile| profile.path.clone())
            .ok_or_else(|| CoreError::UnknownObject(uuid.to_string()))?;
        self.delete_connection(&path).await
    }

    pub async fn reload_connections(self: &Arc<Self>) -> Result<bool, CoreError> {
        for stored in profile_store::load_profiles(&self.config.state_dir) {
            let existing = self
                .state
                .read()
                .await
                .connections
                .values()
                .find(|profile| profile.uuid == stored.uuid)
                .cloned();
            if let Some(existing) = existing {
                let updated = profile_from_stored(&stored, existing.path.clone());
                self.state
                    .write()
                    .await
                    .connections
                    .insert(existing.path.clone(), updated);
                self.notify_settings_connection_updated(&stored.uuid).await;
            } else {
                let path = {
                    let mut state = self.state.write().await;
                    let path = state.next_connection_path();
                    state
                        .connections
                        .insert(path.clone(), profile_from_stored(&stored, path.clone()));
                    path
                };
                self.register_settings_connection_at(&path, &stored.uuid)
                    .await?;
                self.notify_settings_new(&path).await;
            }
        }
        Ok(true)
    }

    pub async fn set_hostname(&self, hostname: &str) {
        self.state.write().await.hostname = hostname.to_string();
        let Some(bus) = self.bus.get() else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, names::NM_SETTINGS_PATH) else {
            return;
        };
        let mut changed = HashMap::new();
        changed.insert("Hostname".to_string(), owned(hostname.to_string()));
        emit_props(&emitter, names::iface::SETTINGS, changed, Vec::new()).await;
    }

    // ------------------------------------------------------------------
    // Activation
    // ------------------------------------------------------------------

    pub async fn add_and_activate(
        self: &Arc<Self>,
        settings: &SettingsMap,
        device: &OwnedObjectPath,
        specific: &OwnedObjectPath,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), CoreError> {
        let draft =
            profile_store::parse_settings(settings, None).map_err(CoreError::InvalidArgs)?;
        let connection = self.add_connection(draft, true).await?;
        let active = self.activate(&connection, device, specific).await?;
        Ok((connection, active))
    }

    pub async fn activate(
        self: &Arc<Self>,
        connection: &OwnedObjectPath,
        device: &OwnedObjectPath,
        specific: &OwnedObjectPath,
    ) -> Result<OwnedObjectPath, CoreError> {
        let profile = if connection.as_str() == "/" {
            self.ephemeral_profile(specific).await?
        } else {
            self.state
                .read()
                .await
                .connections
                .get(connection)
                .cloned()
                .ok_or_else(|| CoreError::UnknownObject(connection.to_string()))?
        };

        let device = if device.as_str() == "/" {
            self.first_device_for(&profile)
                .await
                .ok_or_else(|| CoreError::UnknownObject("no suitable device".to_string()))?
        } else {
            self.state
                .read()
                .await
                .device_by_path(device)
                .cloned()
                .ok_or_else(|| CoreError::UnknownObject(device.to_string()))?
        };

        match (profile.kind, device.kind) {
            (ProfileKind::Wifi, DeviceKind::Wifi) => {}
            (ProfileKind::Ethernet, DeviceKind::Ethernet) => {
                return Err(CoreError::NotSupported(
                    "wired connection activation is not implemented in this MVP".to_string(),
                ));
            }
            _ => {
                return Err(CoreError::InvalidArgs(
                    "connection type does not match the device".to_string(),
                ));
            }
        }

        let existing = {
            self.state
                .read()
                .await
                .active_for_device(&device.path)
                .map(|active| active.path.clone())
        };
        if let Some(existing) = existing {
            let _ = self.deactivate(&existing).await;
        }

        let (active_path, old_device_state) = {
            let mut state = self.state.write().await;
            let old_device_state = state
                .devices
                .get(&device.ifname)
                .map(|device| device.state)
                .unwrap_or(device_state::UNKNOWN);
            let path = state.next_active_path();
            state.active.insert(
                path.clone(),
                ActiveEntry {
                    path: path.clone(),
                    connection_path: profile.path.clone(),
                    device_path: device.path.clone(),
                    specific_object: specific.clone(),
                    id: profile.id.clone(),
                    uuid: profile.uuid.clone(),
                    state: active_connection_state::ACTIVATING,
                    default: false,
                    ip4_config_path: None,
                },
            );
            if let Some(device) = state.devices.get_mut(&device.ifname) {
                device.active_connection = Some(path.clone());
                device.state = device_state::PREPARE;
                device.state_reason = device_state_reason::NONE;
            }
            state.recompute_manager_state();
            (path, old_device_state)
        };

        if let Some(bus) = self.bus.get()
            && let Err(error) = bus
                .server
                .at(
                    active_path.clone(),
                    ActiveConnectionIface::new(self.clone(), active_path.clone()),
                )
                .await
        {
            let mut state = self.state.write().await;
            state.active.remove(&active_path);
            if let Some(device) = state.devices.get_mut(&device.ifname) {
                device.active_connection = None;
                device.state = device_state::DISCONNECTED;
            }
            state.recompute_manager_state();
            return Err(error.into());
        }

        self.notify_device(
            &device.ifname,
            Some((
                old_device_state,
                device_state::PREPARE,
                device_state_reason::NONE,
            )),
        )
        .await;
        self.notify_manager().await;
        self.notify_active(&active_path).await;

        let core = self.clone();
        let profile_for_task = profile.clone();
        let device_path = device.path.clone();
        let specific = specific.clone();
        let task_active_path = active_path.clone();
        tokio::spawn(async move {
            core.run_activation(task_active_path, profile_for_task, device_path, specific)
                .await;
        });
        Ok(active_path)
    }

    async fn ephemeral_profile(
        self: &Arc<Self>,
        specific: &OwnedObjectPath,
    ) -> Result<ConnectionProfile, CoreError> {
        let ap = self
            .state
            .read()
            .await
            .ap_by_path(specific)
            .cloned()
            .ok_or_else(|| {
                CoreError::InvalidArgs("activating / requires a specific access point".to_string())
            })?;
        let key_mgmt = ap
            .security
            .preferred_key_mgmt()
            .wpa_config_value()
            .to_string();
        let draft = ProfileDraft {
            uuid: Uuid::new_v4().to_string(),
            id: ssid_to_string(&ap.ssid),
            kind: ProfileKind::Wifi,
            ifname: Some(ap.ifname.clone()),
            ssid: ap.ssid.clone(),
            psk: None,
            key_mgmt: Some(key_mgmt),
            autoconnect: false,
            timestamp: unix_now(),
        };
        let path = self.add_connection(draft, false).await?;
        self.state
            .read()
            .await
            .connections
            .get(&path)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(path.to_string()))
    }

    async fn first_device_for(&self, profile: &ConnectionProfile) -> Option<DeviceEntry> {
        let state = self.state.read().await;
        if let Some(ifname) = &profile.ifname
            && let Some(device) = state.devices.get(ifname)
        {
            return Some(device.clone());
        }
        let kind = profile.kind;
        state
            .devices
            .values()
            .filter(|device| match kind {
                ProfileKind::Wifi => device.kind == DeviceKind::Wifi,
                ProfileKind::Ethernet => device.kind == DeviceKind::Ethernet,
            })
            .min_by_key(|device| device.ifindex)
            .cloned()
    }

    async fn run_activation(
        self: Arc<Self>,
        active_path: OwnedObjectPath,
        profile: ConnectionProfile,
        device_path: OwnedObjectPath,
        _specific: OwnedObjectPath,
    ) {
        let Some(device) = self.device_by_path(&device_path).await else {
            return;
        };
        let ifname = device.ifname.clone();
        let key_mgmt = profile
            .key_mgmt
            .clone()
            .unwrap_or_else(|| "NONE".to_string());

        if matches!(key_mgmt.as_str(), "WPA-PSK" | "SAE") && profile.psk.is_none() {
            self.fail_activation(
                &active_path,
                &ifname,
                device_state_reason::NO_SECRETS,
                "a passphrase is required for this network",
            )
            .await;
            return;
        }

        self.set_device_state(&ifname, device_state::CONFIG, device_state_reason::NONE)
            .await;

        let request = ConnectRequest {
            ifname: &ifname,
            ssid: &profile.ssid,
            psk: profile.psk.as_deref(),
            key_mgmt: &key_mgmt,
            hidden: false,
        };
        if let Err(error) = self.backend.connect(&request).await {
            tracing::warn!(%error, ifname, "wpa_supplicant connection failed");
            self.fail_activation(
                &active_path,
                &ifname,
                device_state_reason::SUPPLICANT_FAILED,
                &error.to_string(),
            )
            .await;
            return;
        }

        for attempt in 0..40 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if !self.is_current_active(&active_path).await {
                return;
            }
            match self.backend.status(&ifname).await {
                Ok(status) if status.state.is_connected() => {
                    self.complete_activation(&active_path, &ifname, Some(status))
                        .await;
                    return;
                }
                Ok(status)
                    if attempt >= 6
                        && matches!(status.state, crate::backend::WpaState::Disconnected) =>
                {
                    self.fail_activation(
                        &active_path,
                        &ifname,
                        device_state_reason::SUPPLICANT_DISCONNECT,
                        "wpa_supplicant disconnected during activation",
                    )
                    .await;
                    return;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::debug!(%error, ifname, "status poll failed during activation");
                }
            }
        }
        self.fail_activation(
            &active_path,
            &ifname,
            device_state_reason::SUPPLICANT_TIMEOUT,
            "timed out waiting for wpa_supplicant",
        )
        .await;
    }

    async fn complete_activation(
        self: &Arc<Self>,
        active_path: &OwnedObjectPath,
        ifname: &str,
        status: Option<WifiStatus>,
    ) {
        if !self.is_current_active(active_path).await {
            return;
        }
        let ip4 = self.ip4_for(ifname);
        if ip4.is_some() {
            let mut state = self.state.write().await;
            if let Some(device) = state.devices.get_mut(ifname) {
                device.ip4 = ip4.clone();
            }
        }
        let ip4_path = self.ensure_ip4_config(ifname).await;
        let active_ap = status
            .as_ref()
            .and_then(|status| status.bssid.as_ref())
            .map(|bssid| ApEntry::ap_key(ifname, bssid));

        let old_state = {
            let mut state = self.state.write().await;
            let active_valid = state
                .active
                .get(active_path)
                .map(|active| active.state != active_connection_state::DEACTIVATED)
                .unwrap_or(false);
            if !active_valid {
                return;
            }
            let Some(device) = state.devices.get_mut(ifname) else {
                return;
            };
            let old = device.state;
            device.state = device_state::ACTIVATED;
            device.state_reason = device_state_reason::NONE;
            device.ip4 = ip4;
            device.ip4_config_path = ip4_path.clone();
            if active_ap.is_some() {
                device.active_ap = active_ap.clone();
            }
            if let Some(active) = state.active.get_mut(active_path) {
                active.state = active_connection_state::ACTIVATED;
                active.default = true;
                active.ip4_config_path = ip4_path.clone();
            }
            state.recompute_manager_state();
            old
        };

        self.notify_device(
            ifname,
            Some((
                old_state,
                device_state::ACTIVATED,
                device_state_reason::NONE,
            )),
        )
        .await;
        self.notify_active(active_path).await;
        self.notify_manager().await;
        self.notify_ip4(ifname).await;
        self.notify_wireless(ifname, &[], &[]).await;
    }

    async fn fail_activation(
        self: &Arc<Self>,
        active_path: &OwnedObjectPath,
        ifname: &str,
        reason: u32,
        message: &str,
    ) {
        tracing::warn!(ifname, reason, message, "activation failed");
        let old_state = {
            let mut state = self.state.write().await;
            if !state.active.contains_key(active_path) {
                return;
            }
            if let Some(active) = state.active.get_mut(active_path) {
                active.state = active_connection_state::DEACTIVATED;
            }
            let old = state.devices.get(ifname).map(|device| device.state);
            if let Some(device) = state.devices.get_mut(ifname) {
                device.state = device_state::FAILED;
                device.state_reason = reason;
            }
            state.recompute_manager_state();
            old
        };
        self.notify_device(
            ifname,
            Some((
                old_state.unwrap_or(device_state::UNKNOWN),
                device_state::FAILED,
                reason,
            )),
        )
        .await;
        self.notify_active(active_path).await;
        self.notify_manager().await;

        if let Some(bus) = self.bus.get() {
            let _ = bus
                .server
                .remove::<ActiveConnectionIface, _>(active_path)
                .await;
        }
        {
            let mut state = self.state.write().await;
            state.active.remove(active_path);
            if let Some(device) = state.devices.get_mut(ifname)
                && device.active_connection.as_ref() == Some(active_path)
            {
                device.active_connection = None;
            }
            state.recompute_manager_state();
        }
        self.notify_manager().await;
    }

    pub async fn deactivate(
        self: &Arc<Self>,
        active_path: &OwnedObjectPath,
    ) -> Result<(), CoreError> {
        let active = self
            .state
            .read()
            .await
            .active
            .get(active_path)
            .cloned()
            .ok_or_else(|| CoreError::UnknownObject(active_path.to_string()))?;
        let device = self
            .state
            .read()
            .await
            .device_by_path(&active.device_path)
            .cloned();

        let ifname = device.as_ref().map(|device| device.ifname.clone());
        {
            let mut state = self.state.write().await;
            if let Some(active) = state.active.get_mut(active_path) {
                active.state = active_connection_state::DEACTIVATING;
            }
            if let Some(ifname) = &ifname
                && let Some(device) = state.devices.get_mut(ifname)
            {
                device.state = device_state::DEACTIVATING;
            }
        }
        self.notify_active(active_path).await;

        if let Some(device) = &device
            && device.kind == DeviceKind::Wifi
            && let Err(error) = self.backend.disconnect(&device.ifname).await
        {
            tracing::debug!(%error, ifname = %device.ifname, "disconnect failed");
        }

        if let Some(ip4_path) = self
            .state
            .read()
            .await
            .active
            .get(active_path)
            .and_then(|active| active.ip4_config_path.clone())
            && let Some(bus) = self.bus.get()
        {
            let _ = bus.server.remove::<Ip4ConfigIface, _>(&ip4_path).await;
        }

        let old_state = {
            let mut state = self.state.write().await;
            state.active.remove(active_path);
            if let Some(ifname) = &ifname
                && let Some(device) = state.devices.get_mut(ifname)
            {
                let old = device.state;
                device.state = device_state::DISCONNECTED;
                device.state_reason = device_state_reason::USER;
                device.active_connection = None;
                device.active_ap = None;
                device.ip4 = None;
                device.ip4_config_path = None;
                old
            } else {
                device_state::UNKNOWN
            }
        };

        if let Some(bus) = self.bus.get() {
            let _ = bus
                .server
                .remove::<ActiveConnectionIface, _>(active_path)
                .await;
        }
        self.state.write().await.recompute_manager_state();
        if let Some(device) = &device {
            self.notify_device(
                &device.ifname,
                Some((
                    old_state,
                    device_state::DISCONNECTED,
                    device_state_reason::USER,
                )),
            )
            .await;
            self.notify_ip4(&device.ifname).await;
        }
        self.notify_manager().await;
        Ok(())
    }

    async fn is_current_active(&self, path: &OwnedObjectPath) -> bool {
        self.state
            .read()
            .await
            .active
            .get(path)
            .map(|active| active.state != active_connection_state::DEACTIVATED)
            .unwrap_or(false)
    }

    async fn set_device_state(&self, ifname: &str, state: u32, reason: u32) {
        let old = {
            let mut guard = self.state.write().await;
            guard.devices.get_mut(ifname).and_then(|device| {
                if device.state == state && device.state_reason == reason {
                    None
                } else {
                    let old = device.state;
                    device.state = state;
                    device.state_reason = reason;
                    Some(old)
                }
            })
        };
        if let Some(old) = old {
            self.notify_device(ifname, Some((old, state, reason))).await;
        }
    }

    async fn device_by_path(&self, path: &OwnedObjectPath) -> Option<DeviceEntry> {
        self.state.read().await.device_by_path(path).cloned()
    }

    // ------------------------------------------------------------------
    // Backend / netlink event pumps
    // ------------------------------------------------------------------

    fn start_backend_events(self: &Arc<Self>) {
        let (tx, mut rx) = mpsc::channel::<BackendEvent>(128);
        self.backend.clone().spawn_events(tx);
        let core = self.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    BackendEvent::ScanResults { ifname } => {
                        tracing::debug!(ifname, "refreshing access points after scan event");
                        if let Err(error) = core.refresh_access_points(&ifname).await {
                            tracing::warn!(%error, ifname, "failed to refresh scan results");
                        }
                    }
                    BackendEvent::StateChanged { ifname, status } => {
                        core.apply_status_event(&ifname, status).await;
                    }
                    BackendEvent::Error { ifname, message } => {
                        tracing::warn!(ifname, message, "wpa_supplicant event error");
                    }
                }
            }
        });
    }

    fn start_netlink(self: &Arc<Self>) {
        let (tx, mut rx) = mpsc::channel(64);
        if let Err(error) = crate::netlink::spawn_monitor(tx) {
            tracing::warn!(%error, "netlink monitor unavailable; polling interface list");
            let core = self.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    let _ = core.refresh_links().await;
                }
            });
            return;
        }
        let core = self.clone();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    crate::netlink::NetlinkEvent::LinkChanged(link) => {
                        if let Err(error) = core.upsert_device(link).await {
                            tracing::debug!(%error, "failed to update link");
                        }
                    }
                    crate::netlink::NetlinkEvent::LinkRemoved { ifname } => {
                        core.remove_device(&ifname).await;
                    }
                    crate::netlink::NetlinkEvent::AddressChanged { ifname } => {
                        core.refresh_device_ip(&ifname).await;
                    }
                }
            }
        });
    }

    async fn refresh_links(self: &Arc<Self>) {
        match tokio::task::spawn_blocking(crate::netlink::dump_links).await {
            Ok(Ok(links)) => {
                for link in links {
                    if let Err(error) = self.upsert_device(link).await {
                        tracing::debug!(%error, "polled link update failed");
                    }
                }
            }
            Ok(Err(error)) => tracing::debug!(%error, "polled netlink dump failed"),
            Err(error) => tracing::debug!(%error, "polled netlink task failed"),
        }
    }

    fn spawn_initial_sync(self: &Arc<Self>) {
        let core = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            let wifi: Vec<String> = core
                .state
                .read()
                .await
                .devices
                .values()
                .filter(|device| device.kind == DeviceKind::Wifi)
                .map(|device| device.ifname.clone())
                .collect();
            for ifname in wifi {
                if let Ok(status) = core.backend.status(&ifname).await {
                    core.apply_status_event(&ifname, status).await;
                }
                if core.config.initial_scan {
                    let _ = core.backend.scan(&ifname).await;
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    if let Err(error) = core.refresh_access_points(&ifname).await {
                        tracing::debug!(%error, ifname, "initial scan results unavailable");
                    }
                }
            }
        });
    }

    async fn apply_status_event(self: &Arc<Self>, ifname: &str, status: WifiStatus) {
        let has_active = {
            let state = self.state.read().await;
            state
                .devices
                .get(ifname)
                .map(|device| device.active_connection.is_some())
                .unwrap_or(false)
        };
        if has_active {
            return;
        }

        let connected = status.state.is_connected();
        let connecting = status.state.is_connecting();
        let active_ap = status
            .bssid
            .as_ref()
            .map(|bssid| ApEntry::ap_key(ifname, bssid));
        let ip4 = if connected {
            self.ip4_for(ifname)
        } else {
            None
        };
        let (old_state, removed_ip4) = {
            let mut state = self.state.write().await;
            let Some(device) = state.devices.get_mut(ifname) else {
                return;
            };
            let old = device.state;
            let mut removed_ip4 = None;
            if connected {
                device.state = device_state::ACTIVATED;
                device.state_reason = device_state_reason::NONE;
                device.ip4 = ip4;
                if active_ap.is_some() {
                    device.active_ap = active_ap.clone();
                }
            } else if connecting {
                device.state = device_state::CONFIG;
            } else {
                device.state = device_state::DISCONNECTED;
                device.ip4 = None;
                device.active_ap = None;
                removed_ip4 = device.ip4_config_path.take();
            }
            state.recompute_manager_state();
            (old, removed_ip4)
        };

        if let Some(path) = removed_ip4
            && let Some(bus) = self.bus.get()
        {
            let _ = bus.server.remove::<Ip4ConfigIface, _>(&path).await;
        }
        if connected {
            self.ensure_ip4_config(ifname).await;
        }
        self.notify_device(
            ifname,
            Some((
                old_state,
                if connected {
                    device_state::ACTIVATED
                } else if connecting {
                    device_state::CONFIG
                } else {
                    device_state::DISCONNECTED
                },
                device_state_reason::NONE,
            )),
        )
        .await;
        self.notify_wireless(ifname, &[], &[]).await;
        self.notify_manager().await;
    }

    async fn refresh_device_ip(self: &Arc<Self>, ifname: &str) {
        let ip4 = self.ip4_for(ifname);
        let (removed_ip4, transition) = {
            let mut state = self.state.write().await;
            let Some(device) = state.devices.get_mut(ifname) else {
                return;
            };
            let old_state = device.state;
            device.ip4 = ip4.clone();
            let removed = if ip4.is_some() {
                None
            } else {
                device.ip4_config_path.take()
            };
            if device.kind == DeviceKind::Ethernet && device.active_connection.is_none() {
                device.state = if ip4.is_some() {
                    device_state::ACTIVATED
                } else {
                    device_state::DISCONNECTED
                };
                device.state_reason = device_state_reason::NONE;
            }
            let transition = (old_state != device.state).then_some((
                old_state,
                device.state,
                device.state_reason,
            ));
            state.recompute_manager_state();
            (removed, transition)
        };
        if ip4.is_some() {
            self.ensure_ip4_config(ifname).await;
        } else if let Some(path) = removed_ip4
            && let Some(bus) = self.bus.get()
        {
            let _ = bus.server.remove::<Ip4ConfigIface, _>(&path).await;
        }
        self.notify_device(ifname, transition).await;
        self.notify_ip4(ifname).await;
        self.notify_manager().await;
    }

    async fn ensure_ip4_config(self: &Arc<Self>, ifname: &str) -> Option<OwnedObjectPath> {
        let existing = {
            self.state
                .read()
                .await
                .devices
                .get(ifname)
                .and_then(|device| device.ip4_config_path.clone())
        };
        if existing.is_some() {
            return existing;
        }
        self.state
            .read()
            .await
            .devices
            .get(ifname)
            .and_then(|device| device.ip4.as_ref())?;
        let path = {
            let mut state = self.state.write().await;
            let path = state.next_ip4_path();
            if let Some(device) = state.devices.get_mut(ifname) {
                device.ip4_config_path = Some(path.clone());
            }
            path
        };
        let bus = self.bus.get()?;
        if let Err(error) = bus
            .server
            .at(
                path.clone(),
                Ip4ConfigIface::new(self.clone(), ifname.to_string()),
            )
            .await
        {
            tracing::warn!(%error, ifname, "failed to register IP4Config");
            if let Some(device) = self.state.write().await.devices.get_mut(ifname) {
                device.ip4_config_path = None;
            }
            return None;
        }
        Some(path)
    }

    fn ip4_for(&self, ifname: &str) -> Option<Ip4Info> {
        crate::system::ipv4_addresses(ifname)
            .into_iter()
            .next()
            .map(|address| Ip4Info {
                address: address.address.to_string(),
                prefix: address.prefix,
                gateway: None,
                dns: crate::system::dns_servers(),
            })
    }

    // ------------------------------------------------------------------
    // Manager toggles
    // ------------------------------------------------------------------

    pub async fn set_wireless_enabled(self: &Arc<Self>, enabled: bool) {
        {
            let mut state = self.state.write().await;
            state.wireless_enabled = enabled;
        }
        if !enabled {
            let wifi_active: Vec<OwnedObjectPath> = {
                let state = self.state.read().await;
                state
                    .active
                    .values()
                    .filter(|active| {
                        state
                            .device_by_path(&active.device_path)
                            .map(|device| device.kind == DeviceKind::Wifi)
                            .unwrap_or(false)
                    })
                    .map(|active| active.path.clone())
                    .collect()
            };
            for active in wifi_active {
                let _ = self.deactivate(&active).await;
            }
        }
        self.notify_manager().await;
    }

    pub async fn set_networking_enabled(self: &Arc<Self>, enabled: bool) {
        {
            let mut state = self.state.write().await;
            state.networking_enabled = enabled;
            state.recompute_manager_state();
        }
        if !enabled {
            let active_paths: Vec<OwnedObjectPath> = self.state.read().await.active_paths();
            for active in active_paths {
                let _ = self.deactivate(&active).await;
            }
        }
        self.notify_manager().await;
    }

    // ------------------------------------------------------------------
    // Signal emission
    //
    // These helpers deliberately build PropertiesChanged and the NM-specific
    // signals directly from SignalEmitter. Going through
    // ObjectServer::interface(..).get_mut() from inside a method handler
    // deadlocks: the dispatcher already holds a read lock on that interface
    // while the handler is running. Direct emission is also cheaper.
    // ------------------------------------------------------------------

    async fn notify_manager(&self) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let (
            state_value,
            connectivity,
            networking,
            wireless,
            hardware,
            devices,
            active,
            primary,
            activating,
        ) = {
            let state = self.state.read().await;
            (
                state.manager_state,
                state.connectivity,
                state.networking_enabled,
                state.wireless_enabled,
                state.devices.values().any(|device| device.kind.is_wifi()),
                state.device_paths(),
                state.active_paths(),
                state.primary_connection.clone().unwrap_or_else(root_path),
                state
                    .active
                    .values()
                    .find(|active| active.is_activating())
                    .map(|active| active.path.clone())
                    .unwrap_or_else(root_path),
            )
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, names::NM_PATH) else {
            return;
        };
        let mut changed = HashMap::new();
        changed.insert("State".to_string(), owned(state_value));
        changed.insert("Connectivity".to_string(), owned(connectivity));
        changed.insert("NetworkingEnabled".to_string(), owned(networking));
        changed.insert("WirelessEnabled".to_string(), owned(wireless));
        changed.insert("WirelessHardwareEnabled".to_string(), owned(hardware));
        changed.insert("Devices".to_string(), owned(devices));
        changed.insert("ActiveConnections".to_string(), owned(active));
        changed.insert("PrimaryConnection".to_string(), owned(primary));
        changed.insert("ActivatingConnection".to_string(), owned(activating));
        emit_props(&emitter, names::iface::MANAGER, changed, Vec::new()).await;
        let _ = emitter
            .emit(names::iface::MANAGER, "StateChanged", &(state_value,))
            .await;
    }

    async fn notify_device(&self, ifname: &str, transition: Option<(u32, u32, u32)>) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let snapshot = {
            let state = self.state.read().await;
            state.devices.get(ifname).map(|device| {
                (
                    device.path.clone(),
                    device.state,
                    device.state_reason,
                    device.active_connection.clone().unwrap_or_else(root_path),
                    device.ip4_config_path.clone().unwrap_or_else(root_path),
                    device
                        .ip4
                        .as_ref()
                        .and_then(|ip4| ip4.address.parse::<std::net::Ipv4Addr>().ok())
                        .map(u32::from)
                        .unwrap_or(0),
                    device.mtu,
                )
            })
        };
        let Some((path, state_value, reason, active, ip4_config, ip4_address, mtu)) = snapshot
        else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, &path) else {
            return;
        };
        let mut changed = HashMap::new();
        changed.insert("State".to_string(), owned(state_value));
        changed.insert("StateReason".to_string(), owned((state_value, reason)));
        changed.insert("ActiveConnection".to_string(), owned(active));
        changed.insert("Ip4Config".to_string(), owned(ip4_config));
        changed.insert("Ip4Address".to_string(), owned(ip4_address));
        changed.insert("Mtu".to_string(), owned(mtu));
        emit_props(&emitter, names::iface::DEVICE, changed, Vec::new()).await;
        if let Some((old, new, reason)) = transition {
            let _ = emitter
                .emit(names::iface::DEVICE, "StateChanged", &(new, old, reason))
                .await;
        }
    }

    async fn notify_access_point(&self, path: &OwnedObjectPath) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let snapshot = {
            let state = self.state.read().await;
            state.ap_by_path(path).map(|ap| {
                (
                    ap.ssid.clone(),
                    ap.strength,
                    ap.flags,
                    ap.wpa_flags,
                    ap.rsn_flags,
                    ap.last_seen,
                    ap.frequency,
                )
            })
        };
        let Some((ssid, strength, flags, wpa_flags, rsn_flags, last_seen, frequency)) = snapshot
        else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, path) else {
            return;
        };
        let mut changed = HashMap::new();
        changed.insert("Ssid".to_string(), owned(ssid));
        changed.insert("Strength".to_string(), owned(strength));
        changed.insert("Flags".to_string(), owned(flags));
        changed.insert("WpaFlags".to_string(), owned(wpa_flags));
        changed.insert("RsnFlags".to_string(), owned(rsn_flags));
        changed.insert("LastSeen".to_string(), owned(last_seen));
        changed.insert("Frequency".to_string(), owned(frequency));
        emit_props(&emitter, names::iface::ACCESS_POINT, changed, Vec::new()).await;
    }

    async fn notify_wireless(
        &self,
        ifname: &str,
        added: &[OwnedObjectPath],
        removed: &[OwnedObjectPath],
    ) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let snapshot = {
            let state = self.state.read().await;
            state.devices.get(ifname).map(|device| {
                let aps = state.ap_paths(ifname);
                let active_ap = device
                    .active_ap
                    .as_ref()
                    .and_then(|key| state.aps.get(key).map(|ap| ap.path.clone()))
                    .unwrap_or_else(root_path);
                (device.path.clone(), aps, active_ap, device.last_scan)
            })
        };
        let Some((path, aps, active_ap, last_scan)) = snapshot else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, &path) else {
            return;
        };
        let mut changed = HashMap::new();
        changed.insert("AccessPoints".to_string(), owned(aps));
        changed.insert("ActiveAccessPoint".to_string(), owned(active_ap));
        changed.insert("LastScan".to_string(), owned(last_scan));
        emit_props(&emitter, names::iface::DEVICE_WIRELESS, changed, Vec::new()).await;
        for ap in added {
            let _ = emitter
                .emit(
                    names::iface::DEVICE_WIRELESS,
                    "AccessPointAdded",
                    &(ap.clone(),),
                )
                .await;
        }
        for ap in removed {
            let _ = emitter
                .emit(
                    names::iface::DEVICE_WIRELESS,
                    "AccessPointRemoved",
                    &(ap.clone(),),
                )
                .await;
        }
    }

    async fn notify_active(&self, path: &OwnedObjectPath) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let snapshot = {
            let state = self.state.read().await;
            state.active.get(path).map(|active| {
                (
                    active.state,
                    active.default,
                    active.ip4_config_path.clone().unwrap_or_else(root_path),
                    vec![active.device_path.clone()],
                )
            })
        };
        let Some((state_value, default, ip4_config, devices)) = snapshot else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, path) else {
            return;
        };
        let mut changed = HashMap::new();
        changed.insert("State".to_string(), owned(state_value));
        changed.insert("Default".to_string(), owned(default));
        changed.insert("Ip4Config".to_string(), owned(ip4_config));
        changed.insert("Devices".to_string(), owned(devices));
        emit_props(
            &emitter,
            names::iface::ACTIVE_CONNECTION,
            changed,
            Vec::new(),
        )
        .await;
    }

    async fn notify_ip4(&self, ifname: &str) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let snapshot = {
            let state = self.state.read().await;
            state.devices.get(ifname).and_then(|device| {
                device
                    .ip4_config_path
                    .clone()
                    .map(|path| (path, device.ip4.clone()))
            })
        };
        let Some((path, info)) = snapshot else {
            return;
        };
        let info = info.unwrap_or_default();
        let Ok(emitter) = SignalEmitter::new(&bus.connection, &path) else {
            return;
        };
        let address_data = if info.address.is_empty() {
            Vec::new()
        } else {
            vec![HashMap::from([
                ("address".to_string(), owned(info.address.clone())),
                ("prefix".to_string(), owned(u32::from(info.prefix))),
            ])]
        };
        let addresses = info
            .address
            .parse::<std::net::Ipv4Addr>()
            .ok()
            .map(|address| vec![vec![u32::from(address), u32::from(info.prefix)]])
            .unwrap_or_default();
        let nameserver_data: Vec<HashMap<String, OwnedVariant>> = info
            .dns
            .iter()
            .map(|server| HashMap::from([("address".to_string(), owned(server.clone()))]))
            .collect();
        let nameservers: Vec<u32> = info
            .dns
            .iter()
            .filter_map(|server| server.parse::<std::net::Ipv4Addr>().ok())
            .map(u32::from)
            .collect();
        let mut changed = HashMap::new();
        changed.insert("AddressData".to_string(), owned(address_data));
        changed.insert("Addresses".to_string(), owned(addresses));
        changed.insert(
            "Gateway".to_string(),
            owned(info.gateway.unwrap_or_default()),
        );
        changed.insert("NameserverData".to_string(), owned(nameserver_data));
        changed.insert("Nameservers".to_string(), owned(nameservers));
        changed.insert("Domains".to_string(), owned(Vec::<String>::new()));
        changed.insert(
            "Routes".to_string(),
            owned(Vec::<HashMap<String, OwnedVariant>>::new()),
        );
        emit_props(&emitter, names::iface::IP4_CONFIG, changed, Vec::new()).await;
    }

    async fn notify_settings_new(&self, path: &OwnedObjectPath) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, names::NM_SETTINGS_PATH) else {
            return;
        };
        let _ = emitter
            .emit(names::iface::SETTINGS, "NewConnection", &(path.clone(),))
            .await;
    }

    async fn notify_settings_removed(&self, path: &OwnedObjectPath) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, names::NM_SETTINGS_PATH) else {
            return;
        };
        let _ = emitter
            .emit(
                names::iface::SETTINGS,
                "ConnectionRemoved",
                &(path.clone(),),
            )
            .await;
    }

    async fn notify_settings_connection_updated(&self, uuid: &str) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let path = {
            self.state
                .read()
                .await
                .connections
                .values()
                .find(|profile| profile.uuid == uuid)
                .map(|profile| profile.path.clone())
        };
        let Some(path) = path else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, &path) else {
            return;
        };
        let _ = emitter
            .emit(names::iface::SETTINGS_CONNECTION, "Updated", &())
            .await;
    }

    async fn notify_settings_connection_removed(&self, uuid: &str) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let path = {
            self.state
                .read()
                .await
                .connections
                .values()
                .find(|profile| profile.uuid == uuid)
                .map(|profile| profile.path.clone())
        };
        let Some(path) = path else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, &path) else {
            return;
        };
        let _ = emitter
            .emit(names::iface::SETTINGS_CONNECTION, "Removed", &())
            .await;
    }

    async fn notify_device_added(&self, path: &OwnedObjectPath) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, names::NM_PATH) else {
            return;
        };
        let _ = emitter
            .emit(names::iface::MANAGER, "DeviceAdded", &(path.clone(),))
            .await;
    }

    async fn notify_device_removed(&self, path: &OwnedObjectPath) {
        let Some(bus) = self.bus.get() else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(&bus.connection, names::NM_PATH) else {
            return;
        };
        let _ = emitter
            .emit(names::iface::MANAGER, "DeviceRemoved", &(path.clone(),))
            .await;
    }
}

async fn emit_props(
    emitter: &SignalEmitter<'_>,
    interface: &str,
    changed: HashMap<String, OwnedVariant>,
    invalidated: Vec<String>,
) {
    let _ = emitter
        .emit(
            names::iface::PROPERTIES,
            "PropertiesChanged",
            &(interface, changed, invalidated),
        )
        .await;
}

fn profile_from_stored(stored: &StoredProfile, path: OwnedObjectPath) -> ConnectionProfile {
    ConnectionProfile {
        path,
        uuid: stored.uuid.clone(),
        id: stored.id.clone(),
        kind: stored.kind(),
        ifname: stored.ifname.clone(),
        ssid: stored.ssid.clone(),
        psk: stored.psk.clone(),
        key_mgmt: stored.key_mgmt.clone(),
        autoconnect: stored.autoconnect,
        timestamp: stored.timestamp,
    }
}

fn fallback_links() -> Vec<crate::netlink::LinkInfo> {
    crate::system::interface_names()
        .into_iter()
        .map(|ifname| crate::netlink::LinkInfo {
            ifindex: crate::system::ifindex(&ifname).unwrap_or(0),
            ifname,
            mtu: 0,
            flags: 0,
            operstate: 0,
            mac: String::new(),
        })
        .collect()
}

fn is_ignored_interface(ifname: &str) -> bool {
    ifname == "lo0"
        || ifname.starts_with("lo")
        || ifname.starts_with("pflog")
        || ifname.starts_with("pfsync")
        || ifname.starts_with("enc")
        || ifname.starts_with("bridge")
        || ifname.starts_with("vlan")
        || ifname.starts_with("tap")
        || ifname.starts_with("tun")
        || ifname.starts_with("epair")
        || ifname.starts_with("vmnet")
        || ifname.starts_with("usbus")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
