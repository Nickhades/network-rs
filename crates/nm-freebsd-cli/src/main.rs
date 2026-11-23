//! Thin CLI for the nm-freebsd NetworkManager D-Bus service.
//!
//! This binary intentionally talks only to org.freedesktop.NetworkManager; it
//! does not open wpa_supplicant sockets or run dhclient itself.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use nm_common::ssid::ssid_to_string;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Variant};

type SettingsMap = HashMap<String, HashMap<String, OwnedVariant>>;

fn owned<T>(value: T) -> OwnedVariant
where
    Variant<'static>: From<T>,
{
    OwnedVariant::try_from(Variant::from(value)).expect("owned variant")
}

#[derive(Debug, Parser)]
#[command(name = "nm-freebsd-cli", about = "FreeBSD NetworkManager D-Bus CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show manager, device and active-connection state.
    Status,
    /// List devices.
    Devices,
    /// List, show or delete saved connection profiles.
    Connections {
        /// Delete the profile matching this path, UUID or id.
        #[arg(long)]
        delete: Option<String>,
        /// Show the settings of the profile matching this path, UUID or id.
        #[arg(long)]
        show: Option<String>,
        /// Activate the profile matching this path, UUID or id.
        #[arg(long)]
        activate: Option<String>,
    },
    /// Diagnose interfaces, visible bands and driver capabilities.
    Doctor,
    /// Wi-Fi operations.
    Wifi {
        #[command(subcommand)]
        command: WifiCommand,
    },
    /// Disconnect a Wi-Fi device through Device.Disconnect.
    Disconnect {
        /// Interface name, for example wlan0.
        #[arg(long)]
        interface: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum WifiCommand {
    /// List cached access points.
    List {
        /// Request a fresh scan first.
        #[arg(long)]
        scan: bool,
    },
    /// Connect to an SSID (strongest matching AP).
    Connect {
        ssid: String,
        #[arg(long)]
        password: Option<String>,
        /// SSID is not broadcast; tell wpa_supplicant to probe for it.
        #[arg(long)]
        hidden: bool,
    },
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> zbus::Result<OwnedObjectPath>;
    fn add_and_activate_connection(
        &self,
        connection: SettingsMap,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;
    fn add_and_activate_connection2(
        &self,
        connection: SettingsMap,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
        options: HashMap<String, OwnedVariant>,
    ) -> zbus::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedVariant>,
    )>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait Device {
    fn disconnect(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait Wireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(&self, options: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn frequency(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait SettingsConnection {
    fn get_settings(&self) -> zbus::Result<SettingsMap>;
    fn delete(&self) -> zbus::Result<()>;
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let conn = Connection::system()
        .await
        .context("cannot connect to the system D-Bus")?;
    let manager = ManagerProxy::new(&conn)
        .await
        .context("NetworkManager D-Bus service is not reachable")?;

    match cli.command {
        Command::Status => status(&conn, &manager).await?,
        Command::Devices => devices(&conn, &manager).await?,
        Command::Connections {
            delete,
            show,
            activate,
        } => {
            connections(
                &conn,
                &manager,
                delete.as_deref(),
                show.as_deref(),
                activate.as_deref(),
            )
            .await?
        }
        Command::Doctor => doctor(&conn, &manager).await?,
        Command::Wifi { command } => match command {
            WifiCommand::List { scan } => wifi_list(&conn, &manager, scan).await?,
            WifiCommand::Connect {
                ssid,
                password,
                hidden,
            } => wifi_connect(&conn, &manager, &ssid, password.as_deref(), hidden).await?,
        },
        Command::Disconnect { interface } => {
            disconnect(&conn, &manager, interface.as_deref()).await?
        }
    }
    Ok(())
}

fn manager_state(value: u32) -> &'static str {
    match value {
        10 => "asleep",
        20 => "disconnected",
        30 => "disconnecting",
        40 => "connecting",
        50 => "connected-site",
        60 => "connected-global",
        70 => "connected-local",
        _ => "unknown",
    }
}

fn device_state(value: u32) -> &'static str {
    match value {
        10 => "unmanaged",
        20 => "unavailable",
        30 => "disconnected",
        40 => "prepare",
        50 => "config",
        60 => "need-auth",
        70 => "ip-config",
        80 => "ip-check",
        90 => "secondaries",
        100 => "activated",
        110 => "deactivating",
        120 => "failed",
        _ => "unknown",
    }
}

async fn status(conn: &Connection, manager: &ManagerProxy<'_>) -> Result<()> {
    println!("version: {}", manager.version().await?);
    println!(
        "state:   {} ({})",
        manager.state().await?,
        manager_state(manager.state().await?)
    );
    let active = manager.active_connections().await?;
    println!("active:  {} connection(s)", active.len());
    devices(conn, manager).await?;
    Ok(())
}

async fn devices(conn: &Connection, manager: &ManagerProxy<'_>) -> Result<()> {
    for path in manager.get_devices().await? {
        let device = DeviceProxy::builder(conn)
            .path(path.clone())
            .build()
            .await?;
        let iface = device.interface().await.unwrap_or_default();
        let state = device.state().await.unwrap_or(0);
        let kind = match device.device_type().await.unwrap_or(0) {
            1 => "ethernet",
            2 => "wifi",
            _ => "other",
        };
        println!("{iface:12} {kind:8} {} ({state})", device_state(state));
    }
    Ok(())
}

fn setting_string(settings: &SettingsMap, section: &str, key: &str) -> Option<String> {
    settings
        .get(section)
        .and_then(|values| values.get(key))
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| String::try_from(value).ok())
}

async fn connections(
    conn: &Connection,
    manager: &ManagerProxy<'_>,
    delete: Option<&str>,
    show: Option<&str>,
    activate: Option<&str>,
) -> Result<()> {
    let settings = SettingsProxy::new(conn).await?;
    let paths = settings.list_connections().await?;
    let count = paths.len();
    let selector = delete.or(show).or(activate);
    let mut matched = 0;
    let mut selected = None;
    for path in paths {
        let profile = SettingsConnectionProxy::builder(conn)
            .path(path.clone())
            .build()
            .await?;
        let values = profile.get_settings().await?;
        let id = setting_string(&values, "connection", "id").unwrap_or_default();
        let uuid = setting_string(&values, "connection", "uuid").unwrap_or_default();
        let is_match = match selector {
            None => false,
            Some(selector) => selector == path.as_str() || selector == uuid || selector == id,
        };
        if selector.is_none() {
            println!("{id:32} {uuid}  {path}");
            continue;
        }
        if !is_match {
            continue;
        }
        matched += 1;
        selected = Some(path.clone());
        if show.is_some() {
            println!("{id} ({uuid}) {path}");
            println!("{values:#?}");
        }
        if delete.is_some() {
            profile.delete().await?;
            println!("deleted {id} ({uuid})");
        }
    }
    if selector.is_some() && matched == 0 {
        return Err(anyhow!("no profile matched the selector"));
    }
    if let Some(path) = selected
        && activate.is_some()
    {
        let (device, _device_proxy, _wireless) = wifi_device(conn, manager).await?;
        manager
            .activate_connection(path.clone(), device, OwnedObjectPath::try_from("/")?)
            .await?;
        println!("activating {path}");
    }
    if selector.is_none() {
        println!("{count} saved connection(s)");
    }
    Ok(())
}

fn supported_channels(ifname: &str) -> Vec<u32> {
    let output = std::process::Command::new("ifconfig")
        .args([ifname, "list", "chan"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut frequencies = Vec::new();
    for window in tokens.windows(2) {
        if window[1] == "MHz"
            && let Ok(frequency) = window[0].trim_end_matches('*').parse::<u32>()
        {
            frequencies.push(frequency);
        }
    }
    frequencies.sort_unstable();
    frequencies.dedup();
    frequencies
}

fn sysctl_value(name: &str) -> Option<String> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn channel_summary(frequencies: &[u32]) -> String {
    if frequencies.is_empty() {
        return "none".to_string();
    }
    frequencies
        .iter()
        .map(|frequency| frequency.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

async fn doctor(conn: &Connection, manager: &ManagerProxy<'_>) -> Result<()> {
    println!("== daemon ==");
    println!("version: {}", manager.version().await?);
    let state = manager.state().await?;
    println!("state:   {state} ({})", manager_state(state));
    println!(
        "active:  {} connection(s)",
        manager.active_connections().await?.len()
    );

    println!("== interfaces ==");
    devices(conn, manager).await?;

    println!("== wi-fi ==");
    let Ok((_path, device, wireless)) = wifi_device(conn, manager).await else {
        println!("no Wi-Fi device");
        return Ok(());
    };
    let ifname = device.interface().await.unwrap_or_default();
    let aps = access_points(conn, &wireless).await?;
    let mut bands: HashMap<String, usize> = HashMap::new();
    let mut bssids = 0;
    let mut active = None;
    for ap in &aps {
        *bands.entry(band_label(ap.frequency)).or_default() += 1;
        bssids += ap.bssids;
        if ap.active {
            active = Some(ap);
        }
    }
    println!("interface: {ifname}");
    println!("visible:   {} SSID(s), {bssids} BSSID(s)", aps.len());
    for band in ["2.4G", "5G", "6G"] {
        println!(
            "  {band:4} {} SSID(s)",
            bands.get(band).copied().unwrap_or(0)
        );
    }
    match active {
        Some(ap) => println!(
            "active:    {} ({}, {}%)",
            ap.ssid,
            band_label(ap.frequency),
            ap.strength
        ),
        None => println!("active:    none"),
    }

    let channels = supported_channels(&ifname);
    let ch24: Vec<u32> = channels
        .iter()
        .copied()
        .filter(|f| (2400..=2500).contains(f))
        .collect();
    let ch5: Vec<u32> = channels
        .iter()
        .copied()
        .filter(|f| (4900..=5925).contains(f))
        .collect();
    let ch6: Vec<u32> = channels
        .iter()
        .copied()
        .filter(|f| (5925..=7125).contains(f))
        .collect();
    println!("supported channels:");
    println!("  2.4G: {}", channel_summary(&ch24));
    println!("  5G:   {}", channel_summary(&ch5));
    println!("  6G:   {}", channel_summary(&ch6));

    let mut warnings = Vec::new();
    if ch5.is_empty() {
        warnings.push("driver/regulatory exposes no 5GHz channels");
    } else if bands.get("5G").copied().unwrap_or(0) == 0 {
        warnings.push("5GHz channels exist but no 5GHz BSS is visible (channel/range/hidden AP)");
    }
    if bands.get("6G").copied().unwrap_or(0) == 0 && ch6.is_empty() {
        warnings.push("no 6GHz channels or BSS visible");
    }
    let ht = sysctl_value("compat.linuxkpi.rtw89_support_ht");
    let vht = sysctl_value("compat.linuxkpi.rtw89_support_vht");
    if ht.as_deref() == Some("0") || vht.as_deref() == Some("0") {
        warnings.push("rtw89 HT/VHT are disabled; 5GHz support is experimental");
    }
    if !warnings.is_empty() {
        println!("warnings:");
        for warning in warnings {
            println!("  - {warning}");
        }
    }
    println!("hint: run nm-freebsd-cli wifi list --scan for a fresh scan");
    Ok(())
}

async fn wifi_device(
    conn: &Connection,
    manager: &ManagerProxy<'_>,
) -> Result<(
    OwnedObjectPath,
    DeviceProxy<'static>,
    WirelessProxy<'static>,
)> {
    for path in manager.get_devices().await? {
        let device = DeviceProxy::builder(conn)
            .path(path.clone())
            .build()
            .await?;
        if device.device_type().await.unwrap_or(0) != 2 {
            continue;
        }
        let wireless = WirelessProxy::builder(conn)
            .path(path.clone())
            .build()
            .await?;
        return Ok((path, device, wireless));
    }
    Err(anyhow!("no Wi-Fi device"))
}

struct ApInfo {
    path: OwnedObjectPath,
    ssid: String,
    frequency: u32,
    strength: u8,
    secure: bool,
    active: bool,
    bssids: usize,
}

fn band_label(frequency: u32) -> String {
    match frequency {
        2400..=2500 => "2.4G".to_string(),
        4900..=5924 => "5G".to_string(),
        5925..=7125 => "6G".to_string(),
        _ => format!("{frequency}MHz"),
    }
}

async fn access_points(conn: &Connection, wireless: &WirelessProxy<'_>) -> Result<Vec<ApInfo>> {
    let active = wireless.active_access_point().await.ok();
    let mut by_ssid: HashMap<String, ApInfo> = HashMap::new();
    for path in wireless.get_all_access_points().await? {
        let ap = AccessPointProxy::builder(conn)
            .path(path.clone())
            .build()
            .await?;
        let ssid = ssid_to_string(&ap.ssid().await.unwrap_or_default());
        let flags = ap.flags().await.unwrap_or(0);
        let wpa = ap.wpa_flags().await.unwrap_or(0);
        let rsn = ap.rsn_flags().await.unwrap_or(0);
        let strength = ap.strength().await.unwrap_or(0);
        let frequency = ap.frequency().await.unwrap_or(0);
        let secure = (flags & 1) != 0 || wpa != 0 || rsn != 0;
        let is_active = active.as_ref().map(|p| p == &path).unwrap_or(false);

        match by_ssid.get_mut(&ssid) {
            Some(existing) => {
                existing.bssids += 1;
                if is_active {
                    existing.path = path;
                    existing.frequency = frequency;
                    existing.strength = strength;
                    existing.secure = secure;
                    existing.active = true;
                } else if !existing.active && strength > existing.strength {
                    existing.path = path;
                    existing.frequency = frequency;
                    existing.strength = strength;
                    existing.secure = secure;
                }
            }
            None => {
                by_ssid.insert(
                    ssid.clone(),
                    ApInfo {
                        path,
                        ssid,
                        frequency,
                        strength,
                        secure,
                        active: is_active,
                        bssids: 1,
                    },
                );
            }
        }
    }
    let mut result: Vec<_> = by_ssid.into_values().collect();
    result.sort_by_key(|ap| (std::cmp::Reverse(ap.active), std::cmp::Reverse(ap.strength)));
    Ok(result)
}

async fn wifi_list(conn: &Connection, manager: &ManagerProxy<'_>, scan: bool) -> Result<()> {
    let (_path, _device, wireless) = wifi_device(conn, manager).await?;
    if scan {
        wireless.request_scan(HashMap::new()).await?;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    for ap in access_points(conn, &wireless).await? {
        let lock = if ap.secure { "locked" } else { "open" };
        let active = if ap.active { "  [active]" } else { "" };
        println!(
            "{:32} {:5} {:3}%  {:6}  {} BSSID(s){}",
            ap.ssid,
            band_label(ap.frequency),
            ap.strength,
            lock,
            ap.bssids,
            active
        );
    }
    Ok(())
}

async fn wifi_connect(
    conn: &Connection,
    manager: &ManagerProxy<'_>,
    ssid: &str,
    password: Option<&str>,
    hidden: bool,
) -> Result<()> {
    let (device_path, _device, wireless) = wifi_device(conn, manager).await?;
    let (ap_path, secure) = if hidden {
        // A hidden SSID is absent from the scan, so activate against the
        // device with an empty specific object and let the profile set
        // scan_ssid=1 in wpa_supplicant.
        (OwnedObjectPath::try_from("/")?, false)
    } else {
        let aps = access_points(conn, &wireless).await?;
        let Some(ap) = aps.into_iter().find(|ap| ap.ssid == ssid) else {
            return Err(anyhow!(
                "SSID {ssid:?} was not found; run wifi list --scan, or use --hidden"
            ));
        };
        (ap.path, ap.secure)
    };
    if !hidden && secure && password.is_none() {
        return Err(anyhow!("SSID {ssid:?} requires --password"));
    }

    let mut connection = HashMap::new();
    connection.insert("id".to_string(), owned(ssid.to_string()));
    connection.insert("uuid".to_string(), owned(uuid::Uuid::new_v4().to_string()));
    connection.insert("type".to_string(), owned("802-11-wireless".to_string()));

    let mut wireless_setting = HashMap::new();
    wireless_setting.insert("ssid".to_string(), owned(ssid.as_bytes().to_vec()));
    wireless_setting.insert("mode".to_string(), owned("infrastructure".to_string()));
    wireless_setting.insert("hidden".to_string(), owned(hidden));

    let mut settings = HashMap::new();
    settings.insert("connection".to_string(), connection);
    settings.insert("802-11-wireless".to_string(), wireless_setting);
    if let Some(password) = password {
        let mut security = HashMap::new();
        security.insert("key-mgmt".to_string(), owned("wpa-psk".to_string()));
        security.insert("psk".to_string(), owned(password.to_string()));
        settings.insert("802-11-wireless-security".to_string(), security);
    }

    let active = match manager
        .add_and_activate_connection2(
            settings.clone(),
            device_path.clone(),
            ap_path.clone(),
            HashMap::new(),
        )
        .await
    {
        Ok((_connection, active, _result)) => active,
        Err(_) => {
            // Older daemons do not expose AddAndActivateConnection2.
            let (_connection, active) = manager
                .add_and_activate_connection(settings, device_path, ap_path)
                .await?;
            active
        }
    };
    println!("activated: {active}");
    Ok(())
}

async fn disconnect(
    conn: &Connection,
    manager: &ManagerProxy<'_>,
    interface: Option<&str>,
) -> Result<()> {
    let (_path, device, _wireless) = wifi_device(conn, manager).await?;
    if let Some(interface) = interface {
        let current = device.interface().await.unwrap_or_default();
        if current != interface {
            return Err(anyhow!("Wi-Fi device is {current}, not {interface}"));
        }
    }
    device.disconnect().await?;
    println!("disconnected");
    Ok(())
}
