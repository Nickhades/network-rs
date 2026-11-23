//! End-to-end test of the D-Bus surface against a private bus and a fake
//! wpa_supplicant control socket. This does not touch the host's network.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use nm_freebsd_daemon::config::Config;
use nm_freebsd_daemon::daemon;
use tempfile::TempDir;
use zbus::connection::Builder;
use zbus::{OwnedObjectPath, OwnedVariant, Variant};

type SettingsMap = HashMap<String, HashMap<String, OwnedVariant>>;

fn owned<T>(value: T) -> OwnedVariant
where
    Variant<'static>: From<T>,
{
    OwnedVariant::try_from(Variant::from(value)).expect("owned variant")
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn get_permissions(&self) -> zbus::Result<HashMap<String, String>>;
    fn add_and_activate_connection(
        &self,
        connection: SettingsMap,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;
    fn deactivate_connection(&self, active_connection: OwnedObjectPath) -> zbus::Result<()>;
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    #[zbus(signal)]
    fn state_changed(&self, state: u32) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait ManagerState {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait Device {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait Wireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(&self, options: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
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
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait Active {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn get_connection(&self, connection: OwnedObjectPath) -> zbus::Result<SettingsMap>;
}

struct PrivateBus {
    _dir: TempDir,
    child: Child,
    address: String,
}

impl PrivateBus {
    fn start() -> Option<Self> {
        let dir = tempfile::tempdir().ok()?;
        let socket = dir.path().join("bus");
        let services = dir.path().join("services");
        std::fs::create_dir_all(&services).ok()?;
        let config = dir.path().join("system.conf");
        let xml = format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>system</type>
  <listen>unix:path={socket}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <allow own="*"/>
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow receive_type="method_call"/>
    <allow receive_type="signal"/>
    <allow send_type="method_call"/>
    <allow send_type="signal"/>
  </policy>
  <servicedir>{services}</servicedir>
</busconfig>
"#,
            socket = socket.display(),
            services = services.display()
        );
        std::fs::write(&config, xml).ok()?;
        let binary = if Path::new("/usr/local/bin/dbus-daemon").exists() {
            "/usr/local/bin/dbus-daemon"
        } else {
            "dbus-daemon"
        };
        let child = Command::new(binary)
            .arg(format!("--config-file={}", config.display()))
            .arg("--nofork")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        for _ in 0..100 {
            if socket.exists() {
                return Some(Self {
                    _dir: dir,
                    child,
                    address: format!("unix:path={}", socket.display()),
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct FakeWpa {
    dir: TempDir,
    commands: Arc<Mutex<Vec<String>>>,
    selected: Arc<AtomicBool>,
}

impl FakeWpa {
    fn start() -> std::io::Result<Self> {
        let dir = tempfile::tempdir()?;
        let wpa_dir = dir.path().join("wpa");
        std::fs::create_dir_all(&wpa_dir)?;
        let socket = wpa_dir.join("wlan0");
        let listener = UnixDatagram::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o666))?;
        let commands = Arc::new(Mutex::new(Vec::new()));
        let selected = Arc::new(AtomicBool::new(false));
        let commands_thread = commands.clone();
        let selected_thread = selected.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok((n, addr)) = listener.recv_from(&mut buf) {
                let command = String::from_utf8_lossy(&buf[..n]).trim_end().to_string();
                if command.is_empty() {
                    continue;
                }
                commands_thread.lock().unwrap().push(command.clone());
                let response = if command == "SCAN_RESULTS" {
                    String::from(
                        "bssid / frequency / signal level / flags / ssid
aa:bb:cc:dd:ee:01	2412	-42	[WPA2-PSK-CCMP][ESS]	Cafe WiFi
aa:bb:cc:dd:ee:02	5180	-77	[ESS]	Open Net
",
                    )
                } else if command == "STATUS" {
                    if selected_thread.load(Ordering::SeqCst) {
                        String::from(
                            "bssid=aa:bb:cc:dd:ee:01
ssid=Cafe WiFi
wpa_state=COMPLETED
key_mgmt=WPA2-PSK
",
                        )
                    } else {
                        String::from(
                            "wpa_state=DISCONNECTED
",
                        )
                    }
                } else if command == "ADD_NETWORK" {
                    String::from(
                        "0
",
                    )
                } else if command == "SELECT_NETWORK 0" {
                    selected_thread.store(true, Ordering::SeqCst);
                    String::from(
                        "OK
",
                    )
                } else if command == "DISCONNECT" {
                    selected_thread.store(false, Ordering::SeqCst);
                    String::from(
                        "OK
",
                    )
                } else {
                    String::from(
                        "OK
",
                    )
                };
                if let Some(client_path) = addr.as_pathname() {
                    let _ = listener.send_to(response.as_bytes(), client_path);
                }
            }
        });
        Ok(Self {
            dir,
            commands,
            selected,
        })
    }

    fn wpa_dir(&self) -> PathBuf {
        self.dir.path().join("wpa")
    }

    fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }
}

async fn connect(address: &str) -> zbus::Connection {
    Builder::address(address)
        .build()
        .await
        .expect("client bus connection")
}

async fn wifi_device(conn: &zbus::Connection) -> Option<(OwnedObjectPath, DeviceProxy<'static>)> {
    let manager = ManagerProxy::new(conn).await.ok()?;
    for path in manager.get_devices().await.ok()? {
        let device = DeviceProxy::builder(conn)
            .path(path.clone())
            .build()
            .await
            .ok()?;
        if device.device_type().await.ok()? == 2 {
            return Some((path, device));
        }
    }
    None
}

async fn wait_for<F, Fut>(timeout: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mvp_end_to_end() {
    let Some(bus) = PrivateBus::start() else {
        eprintln!("skipping: private dbus-daemon is not available");
        return;
    };
    let fake = FakeWpa::start().expect("fake wpa_supplicant");
    let state_dir = tempfile::tempdir().expect("state dir");
    let mut config = Config::for_tests(fake.wpa_dir(), state_dir.path().to_path_buf());
    config.bus_address = Some(bus.address.clone());
    config.include_wired = false;
    config.initial_scan = true;

    let _running = daemon::start(config).await.expect("daemon starts");
    let conn = connect(&bus.address).await;
    let manager = ManagerProxy::new(&conn).await.expect("manager proxy");
    let permissions = manager.get_permissions().await.expect("permissions");
    assert_eq!(permissions.len(), 11, "{permissions:?}");
    assert_eq!(
        permissions
            .get("org.freedesktop.NetworkManager.wifi-scan")
            .map(String::as_str),
        Some("yes"),
        "{permissions:?}"
    );
    assert!(
        permissions
            .values()
            .all(|value| value == "yes" || value == "no"),
        "{permissions:?}"
    );
    let manager_state = ManagerStateProxy::new(&conn)
        .await
        .expect("manager state proxy");

    let mut found = None;
    for _ in 0..50 {
        if let Some(pair) = wifi_device(&conn).await {
            found = Some(pair);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (device_path, device) = found.expect("wifi device");
    assert_eq!(device.device_type().await.unwrap(), 2);

    let wireless = WirelessProxy::builder(&conn)
        .path(device_path.clone())
        .build()
        .await
        .expect("wireless proxy");

    let mut state_stream = manager
        .receive_state_changed()
        .await
        .expect("state signal stream");

    wireless
        .request_scan(HashMap::new())
        .await
        .expect("request scan");
    let got_aps = wait_for(Duration::from_secs(5), || {
        let wireless = &wireless;
        async move {
            wireless
                .get_all_access_points()
                .await
                .map(|aps| aps.len() == 2)
                .unwrap_or(false)
        }
    })
    .await;
    assert!(got_aps, "scan results did not arrive");
    let aps = wireless.get_all_access_points().await.unwrap();
    let first = AccessPointProxy::builder(&conn)
        .path(aps[0].clone())
        .build()
        .await
        .unwrap();
    assert_eq!(first.ssid().await.unwrap(), b"Cafe WiFi");
    assert!(first.strength().await.unwrap() >= 90);
    assert_eq!(first.flags().await.unwrap() & 1, 1);

    let mut connection = HashMap::new();
    connection.insert("id".to_string(), owned("Cafe WiFi".to_string()));
    connection.insert("uuid".to_string(), owned(uuid::Uuid::new_v4().to_string()));
    connection.insert("type".to_string(), owned("802-11-wireless".to_string()));
    let mut wireless_setting = HashMap::new();
    wireless_setting.insert("ssid".to_string(), owned(b"Cafe WiFi".to_vec()));
    wireless_setting.insert("mode".to_string(), owned("infrastructure".to_string()));
    let mut security = HashMap::new();
    security.insert("key-mgmt".to_string(), owned("wpa-psk".to_string()));
    security.insert("psk".to_string(), owned("hunter2hunter2".to_string()));
    let mut settings = HashMap::new();
    settings.insert("connection".to_string(), connection);
    settings.insert("802-11-wireless".to_string(), wireless_setting);
    settings.insert("802-11-wireless-security".to_string(), security);

    let root = OwnedObjectPath::try_from("/").unwrap();
    let (connection_path, active_path) = manager
        .add_and_activate_connection(settings, device_path.clone(), root.clone())
        .await
        .expect("add and activate");

    let active = ActiveProxy::builder(&conn)
        .path(active_path.clone())
        .build()
        .await
        .unwrap();
    let device = Arc::new(device);
    let active = Arc::new(active);
    let manager_state = Arc::new(manager_state);
    let activated = wait_for(Duration::from_secs(10), || {
        let active = active.clone();
        let device = device.clone();
        let manager_state = manager_state.clone();
        async move {
            active.state().await.unwrap_or(0) == 2
                && device.state().await.unwrap_or(0) == 100
                && manager_state.state().await.unwrap_or(0) == 60
        }
    })
    .await;
    assert!(activated, "activation did not complete");
    assert_eq!(
        manager.active_connections().await.unwrap(),
        vec![active_path.clone()]
    );

    let signal = tokio::time::timeout(Duration::from_secs(5), state_stream.next()).await;
    assert!(
        matches!(signal, Ok(Some(_))),
        "manager did not emit StateChanged"
    );

    let commands = fake.commands();
    assert!(
        commands.iter().any(|command| command == "SELECT_NETWORK 0"),
        "wpa_supplicant was not asked to select the network: {commands:?}"
    );
    assert!(
        commands
            .iter()
            .any(|command| command.starts_with("SET_NETWORK 0 ssid 436166652057694669")),
        "SSID was not sent as hex: {commands:?}"
    );

    let settings_proxy = SettingsProxy::new(&conn).await.unwrap();
    let connections = settings_proxy.list_connections().await.unwrap();
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0], connection_path);
    let stored = settings_proxy
        .get_connection(connection_path.clone())
        .await
        .unwrap();
    let stored_ssid =
        Vec::<u8>::try_from(stored["802-11-wireless"]["ssid"].try_clone().unwrap()).unwrap();
    assert_eq!(stored_ssid, b"Cafe WiFi");

    manager
        .deactivate_connection(active_path.clone())
        .await
        .expect("deactivate");
    let disconnected = wait_for(Duration::from_secs(5), || {
        let manager_state = manager_state.clone();
        let device = device.clone();
        async move {
            manager_state.state().await.unwrap_or(0) == 20
                && device.state().await.unwrap_or(0) == 30
        }
    })
    .await;
    assert!(disconnected, "deactivation did not settle");
    assert!(!fake.selected.load(Ordering::SeqCst));
}
