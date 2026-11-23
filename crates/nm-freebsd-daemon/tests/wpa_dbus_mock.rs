//! Prove the wpa_supplicant D-Bus scan chain:
//! NetworkManager.RequestScan -> Interface.Scan -> ScanDone -> BSSs -> APs.
//!
//! Stock FreeBSD has no wpa_supplicant D-Bus service, so this test provides a
//! small mock of fi.w1.wpa_supplicant1 on a private system bus.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use nm_freebsd_daemon::config::{BackendKind, Config};
use nm_freebsd_daemon::daemon;
use tempfile::TempDir;
use zbus::connection::Builder;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Variant, interface};

const WPA_NAME: &str = "fi.w1.wpa_supplicant1";
const WPA_PATH: &str = "/fi/w1/wpa_supplicant1";
const WPA_IFACE_PATH: &str = "/fi/w1/wpa_supplicant1/Interfaces/0";

fn owned<T>(value: T) -> OwnedVariant
where
    Variant<'static>: From<T>,
{
    OwnedVariant::try_from(Variant::from(value)).expect("owned variant")
}

#[derive(Default)]
struct MockState {
    bsss: HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>>,
    scans: usize,
    scan_done_count: usize,
}

fn bss_map() -> HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>> {
    let path =
        OwnedObjectPath::try_from("/fi/w1/wpa_supplicant1/Interfaces/0/BSS/1").expect("bss path");
    let mut props = HashMap::new();
    props.insert("SSID".to_string(), owned(b"Cafe WiFi".to_vec()));
    props.insert(
        "BSSID".to_string(),
        owned(vec![0xaau8, 0xbb, 0xcc, 0xdd, 0xee, 0x01]),
    );
    props.insert("Frequency".to_string(), owned(2412u16));
    props.insert("Signal".to_string(), owned(-42i16));
    props.insert("Privacy".to_string(), owned(true));
    HashMap::from([(path, props)])
}

struct MockRoot;

#[interface(name = "fi.w1.wpa_supplicant1")]
impl MockRoot {
    fn get_interface(&self, _ifname: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(WPA_IFACE_PATH).unwrap()
    }

    fn create_interface(&self, _args: HashMap<String, OwnedVariant>) -> OwnedObjectPath {
        OwnedObjectPath::try_from(WPA_IFACE_PATH).unwrap()
    }

    #[zbus(property)]
    fn interfaces(&self) -> Vec<OwnedObjectPath> {
        vec![OwnedObjectPath::try_from(WPA_IFACE_PATH).unwrap()]
    }
}

struct MockIface {
    state: Arc<Mutex<MockState>>,
}

#[interface(name = "fi.w1.wpa_supplicant1.Interface")]
impl MockIface {
    async fn scan(
        &self,
        _args: HashMap<String, OwnedVariant>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        {
            let mut state = self.state.lock().expect("mock state");
            state.scans += 1;
        }
        let state = Arc::clone(&self.state);
        let emitter = emitter.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            {
                let mut guard = state.lock().expect("mock state");
                guard.bsss = bss_map();
                guard.scan_done_count += 1;
            }
            let _ = MockIface::scan_done(&emitter, true).await;
        });
    }

    #[zbus(property)]
    fn ifname(&self) -> String {
        "wlan0".to_string()
    }

    #[zbus(property)]
    fn state(&self) -> String {
        "disconnected".to_string()
    }

    #[zbus(property, name = "BSSs")]
    fn bsss(&self) -> HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>> {
        self.state.lock().expect("mock state").bsss.clone()
    }

    #[zbus(property, name = "CurrentBSS")]
    fn current_bss(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/").unwrap()
    }

    #[zbus(signal)]
    async fn scan_done(emitter: &SignalEmitter<'_>, success: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "fi.w1.wpa_supplicant1.Interface",
    default_service = "fi.w1.wpa_supplicant1",
    default_path = "/fi/w1/wpa_supplicant1/Interfaces/0"
)]
trait MockWpaInterface {
    fn scan(&self, args: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
    #[zbus(signal)]
    fn scan_done(&self, success: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait Device {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait Wireless {
    fn request_scan(&self, options: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/"
)]
trait AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
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
    <allow user="*"/><allow own="*"/>
    <allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/>
    <allow receive_type="method_call"/><allow receive_type="signal"/>
    <allow send_type="method_call"/><allow send_type="signal"/>
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

async fn connect(address: &str) -> Connection {
    Builder::address(address)
        .build()
        .await
        .expect("private bus connection")
}

async fn wifi_device(conn: &Connection) -> Option<(OwnedObjectPath, DeviceProxy<'static>)> {
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

async fn wait_for<F, Fut>(limit: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_scan_uses_wpa_dbus_scan_done() {
    let has_wlan = Command::new("ifconfig")
        .arg("wlan0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !has_wlan {
        eprintln!("skipping: wlan0 is not present");
        return;
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter("nm_freebsd_daemon=debug")
        .try_init();
    let Some(bus) = PrivateBus::start() else {
        eprintln!("skipping: private dbus-daemon is not available");
        return;
    };

    let state = Arc::new(Mutex::new(MockState::default()));
    let mock_iface = MockIface {
        state: Arc::clone(&state),
    };
    let mock_connection = Builder::address(bus.address.as_str())
        .name(WPA_NAME)
        .serve_at(WPA_PATH, MockRoot)
        .serve_at(WPA_IFACE_PATH, mock_iface)
        .build()
        .await
        .expect("mock wpa service");
    // Keep the mock connection alive for the whole test.
    let _mock_connection = mock_connection;

    let state_dir = tempfile::tempdir().expect("state dir");
    let mut config =
        Config::for_tests(state_dir.path().join("wpa"), state_dir.path().to_path_buf());
    config.backend = BackendKind::WpaDbus;
    config.bus_address = Some(bus.address.clone());
    config.include_wired = false;
    config.initial_scan = false;
    let _running = daemon::start(config).await.expect("daemon start");

    let conn = connect(&bus.address).await;
    let mut found = None;
    for _ in 0..50 {
        if let Some(pair) = wifi_device(&conn).await {
            found = Some(pair);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (device_path, _device) = found.expect("wlan0 device");
    let wireless = WirelessProxy::builder(&conn)
        .path(device_path)
        .build()
        .await
        .expect("wireless proxy");

    // Let the daemon's ScanDone subscription become active.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Trigger ScanDone directly on the mock; the only path that can populate
    // the NM AP list here is the daemon's ScanDone listener.
    let mock = MockWpaInterfaceProxy::new(&conn)
        .await
        .expect("mock interface proxy");
    let mut mock_signals = mock
        .receive_scan_done()
        .await
        .expect("mock ScanDone subscription");
    mock.scan(HashMap::new()).await.expect("mock scan");
    let signal = tokio::time::timeout(Duration::from_secs(2), mock_signals.next())
        .await
        .expect("mock ScanDone timeout");
    assert!(signal.is_some(), "mock did not emit ScanDone");

    let backend = nm_freebsd_daemon::backend::wpa_dbus::WpaDbusBackend::new(
        vec!["wlan0".to_string()],
        Some(&bus.address),
    )
    .await
    .expect("backend for BSS check");
    use nm_freebsd_daemon::backend::Backend;
    let direct_aps = backend
        .scan_results("wlan0")
        .await
        .expect("direct BSS read");
    assert_eq!(direct_aps.len(), 1, "mock BSSs property is not readable");

    let saw_ap = wait_for(Duration::from_secs(5), || {
        let wireless = &wireless;
        async move {
            wireless
                .get_all_access_points()
                .await
                .map(|aps| !aps.is_empty())
                .unwrap_or(false)
        }
    })
    .await;
    assert!(saw_ap, "ScanDone did not refresh the AP list");

    let aps = wireless.get_all_access_points().await.unwrap();
    let ap = AccessPointProxy::builder(&conn)
        .path(aps[0].clone())
        .build()
        .await
        .expect("AP proxy");
    assert_eq!(ap.ssid().await.unwrap(), b"Cafe WiFi");
    assert_eq!(state.lock().unwrap().scans, 1);

    // The upper-layer RequestScan must reach Interface.Scan.
    wireless
        .request_scan(HashMap::new())
        .await
        .expect("NM RequestScan");
    let scanned = wait_for(Duration::from_secs(3), || {
        let state = Arc::clone(&state);
        async move { state.lock().unwrap().scans >= 2 }
    })
    .await;
    assert!(
        scanned,
        "NetworkManager RequestScan did not reach Interface.Scan"
    );
}
