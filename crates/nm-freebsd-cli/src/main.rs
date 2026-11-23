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
    /// List saved connection profiles.
    Connections,
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
    },
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
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
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
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
        Command::Connections => connections(&conn).await?,
        Command::Wifi { command } => match command {
            WifiCommand::List { scan } => wifi_list(&conn, &manager, scan).await?,
            WifiCommand::Connect { ssid, password } => {
                wifi_connect(&conn, &manager, &ssid, password.as_deref()).await?
            }
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

async fn connections(conn: &Connection) -> Result<()> {
    let settings = SettingsProxy::new(conn).await?;
    let paths = settings.list_connections().await?;
    println!("{} saved connection(s)", paths.len());
    for path in paths {
        println!("{path}");
    }
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

async fn access_points(
    conn: &Connection,
    wireless: &WirelessProxy<'_>,
) -> Result<Vec<(OwnedObjectPath, String, u8, bool)>> {
    let active = wireless.active_access_point().await.ok();
    let mut result = Vec::new();
    for path in wireless.get_all_access_points().await? {
        let ap = AccessPointProxy::builder(conn)
            .path(path.clone())
            .build()
            .await?;
        let ssid = ssid_to_string(&ap.ssid().await.unwrap_or_default());
        let flags = ap.flags().await.unwrap_or(0);
        let wpa = ap.wpa_flags().await.unwrap_or(0);
        let rsn = ap.rsn_flags().await.unwrap_or(0);
        result.push((
            path,
            ssid,
            ap.strength().await.unwrap_or(0),
            (flags & 1) != 0 || wpa != 0 || rsn != 0,
        ));
    }
    result.sort_by_key(|a| std::cmp::Reverse(a.2));
    let _ = active;
    Ok(result)
}

async fn wifi_list(conn: &Connection, manager: &ManagerProxy<'_>, scan: bool) -> Result<()> {
    let (_path, _device, wireless) = wifi_device(conn, manager).await?;
    if scan {
        wireless.request_scan(HashMap::new()).await?;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    for (_path, ssid, strength, secure) in access_points(conn, &wireless).await? {
        let lock = if secure { "locked" } else { "open" };
        println!("{ssid:32} {strength:3}%  {lock}");
    }
    Ok(())
}

async fn wifi_connect(
    conn: &Connection,
    manager: &ManagerProxy<'_>,
    ssid: &str,
    password: Option<&str>,
) -> Result<()> {
    let (device_path, _device, wireless) = wifi_device(conn, manager).await?;
    let aps = access_points(conn, &wireless).await?;
    let Some((ap_path, _ssid, _strength, secure)) =
        aps.into_iter().find(|(_, label, _, _)| label == ssid)
    else {
        return Err(anyhow!("SSID {ssid:?} was not found; run wifi list --scan"));
    };
    if secure && password.is_none() {
        return Err(anyhow!("SSID {ssid:?} requires --password"));
    }

    let mut connection = HashMap::new();
    connection.insert("id".to_string(), owned(ssid.to_string()));
    connection.insert("uuid".to_string(), owned(uuid::Uuid::new_v4().to_string()));
    connection.insert("type".to_string(), owned("802-11-wireless".to_string()));

    let mut wireless_setting = HashMap::new();
    wireless_setting.insert("ssid".to_string(), owned(ssid.as_bytes().to_vec()));
    wireless_setting.insert("mode".to_string(), owned("infrastructure".to_string()));

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
