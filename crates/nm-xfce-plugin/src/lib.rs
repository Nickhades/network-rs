//! XFCE panel plugin for the NetworkManager FreeBSD compatibility service.
//!
//! The panel loads this cdylib in-process. All D-Bus work happens on a
//! dedicated worker thread; GTK is only touched on the main thread. A daemon
//! restart is therefore invisible to the plugin: every request reconnects to
//! the system bus, a signal listener reconnects, and a 30-second fallback poll
//! recovers automatically.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use futures_util::StreamExt;
use nm_common::ssid::ssid_to_string;
use xfce_panel::glib;
use xfce_panel::gtk;
use xfce_panel::gtk::prelude::*;
use xfce_panel::{PanelPlugin, Plugin};
use zbus::connection::Builder as ConnectionBuilder;
use zbus::message::Type as MessageType;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant};

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn add_and_activate_connection(
        &self,
        connection: std::collections::HashMap<
            String,
            std::collections::HashMap<String, OwnedVariant>,
        >,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;
    fn deactivate_connection(&self, active_connection: OwnedObjectPath) -> zbus::Result<()>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
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
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(
        &self,
        options: std::collections::HashMap<String, OwnedVariant>,
    ) -> zbus::Result<()>;
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
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    manager_state: u32,
    connected_ssid: Option<String>,
    aps: Vec<ApInfo>,
}

#[derive(Debug, Clone)]
struct ApInfo {
    ssid: Vec<u8>,
    label: String,
    strength: u8,
    privacy: bool,
}

#[derive(Debug)]
enum Request {
    Refresh,
    Rescan,
    Connect { ssid: Vec<u8>, psk: Option<String> },
    Disconnect,
    Shutdown,
}

#[derive(Debug)]
enum UiEvent {
    Snapshot(Snapshot),
    Busy(bool),
    Error(String),
}

struct BusClient {
    connection: Connection,
}

impl BusClient {
    async fn connect() -> Result<Self, String> {
        let connection = ConnectionBuilder::system()
            .build()
            .await
            .map_err(|error| format!("system bus unavailable: {error}"))?;
        Ok(Self { connection })
    }

    async fn snapshot(&self) -> Result<Snapshot, String> {
        let manager = ManagerProxy::new(&self.connection)
            .await
            .map_err(|error| format!("NetworkManager unavailable: {error}"))?;
        let manager_state = manager
            .state()
            .await
            .map_err(|error| format!("manager state: {error}"))?;

        let mut wifi_path = None;
        for path in manager
            .get_devices()
            .await
            .map_err(|error| format!("device list: {error}"))?
        {
            let device = DeviceProxy::builder(&self.connection)
                .path(path.clone())
                .build()
                .await
                .map_err(|error| format!("device proxy: {error}"))?;
            if device.device_type().await.unwrap_or(0) == 2 {
                wifi_path = Some(path);
                break;
            }
        }
        let Some(wifi_path) = wifi_path else {
            return Err("no Wi-Fi device".to_string());
        };
        let wireless = WirelessProxy::builder(&self.connection)
            .path(wifi_path)
            .build()
            .await
            .map_err(|error| format!("wireless proxy: {error}"))?;

        let active_ap = wireless.active_access_point().await.ok();
        let mut connected_ssid = None;
        if let Some(active_ap) = active_ap
            && active_ap.as_str() != "/"
            && let Ok(ap) = AccessPointProxy::builder(&self.connection)
                .path(active_ap)
                .build()
                .await
        {
            connected_ssid = ap.ssid().await.ok().map(|ssid| ssid_to_string(&ssid));
        }

        let mut by_ssid: std::collections::HashMap<Vec<u8>, ApInfo> =
            std::collections::HashMap::new();
        for path in wireless
            .get_all_access_points()
            .await
            .map_err(|error| format!("access point list: {error}"))?
        {
            let Ok(ap) = AccessPointProxy::builder(&self.connection)
                .path(path)
                .build()
                .await
            else {
                continue;
            };
            let Ok(ssid) = ap.ssid().await else {
                continue;
            };
            if ssid.is_empty() {
                continue;
            }
            let label = ssid_to_string(&ssid);
            let strength = ap.strength().await.unwrap_or(0);
            let privacy = ap.flags().await.unwrap_or(0) & 1 == 1;
            let info = ApInfo {
                ssid,
                label,
                strength,
                privacy,
            };
            by_ssid
                .entry(info.ssid.clone())
                .and_modify(|existing| {
                    if info.strength > existing.strength {
                        *existing = info.clone();
                    }
                })
                .or_insert(info);
        }
        let mut aps: Vec<ApInfo> = by_ssid.into_values().collect();
        aps.sort_by_key(|ap| std::cmp::Reverse(ap.strength));
        aps.truncate(16);

        Ok(Snapshot {
            manager_state,
            connected_ssid,
            aps,
        })
    }

    async fn rescan(&self) -> Result<(), String> {
        let manager = ManagerProxy::new(&self.connection)
            .await
            .map_err(|e| e.to_string())?;
        for path in manager.get_devices().await.map_err(|e| e.to_string())? {
            let device = DeviceProxy::builder(&self.connection)
                .path(path.clone())
                .build()
                .await
                .map_err(|e| e.to_string())?;
            if device.device_type().await.unwrap_or(0) == 2 {
                let wireless = WirelessProxy::builder(&self.connection)
                    .path(path)
                    .build()
                    .await
                    .map_err(|e| e.to_string())?;
                wireless
                    .request_scan(std::collections::HashMap::new())
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
        Err("no Wi-Fi device".to_string())
    }

    async fn connect_to(&self, ssid: &[u8], psk: Option<&str>) -> Result<(), String> {
        let manager = ManagerProxy::new(&self.connection)
            .await
            .map_err(|e| e.to_string())?;
        let mut wifi_path = None;
        for path in manager.get_devices().await.map_err(|e| e.to_string())? {
            let device = DeviceProxy::builder(&self.connection)
                .path(path.clone())
                .build()
                .await
                .map_err(|e| e.to_string())?;
            if device.device_type().await.unwrap_or(0) == 2 {
                wifi_path = Some(path);
                break;
            }
        }
        let Some(wifi_path) = wifi_path else {
            return Err("no Wi-Fi device".to_string());
        };

        let mut connection = std::collections::HashMap::new();
        connection.insert("id".to_string(), owned(ssid_to_string(ssid)));
        connection.insert("uuid".to_string(), owned(uuid_v4()));
        connection.insert("type".to_string(), owned("802-11-wireless".to_string()));
        connection.insert("autoconnect".to_string(), owned(true));

        let mut wireless_setting = std::collections::HashMap::new();
        wireless_setting.insert("ssid".to_string(), owned(ssid.to_vec()));
        wireless_setting.insert("mode".to_string(), owned("infrastructure".to_string()));

        let mut settings = std::collections::HashMap::new();
        settings.insert("connection".to_string(), connection);
        settings.insert("802-11-wireless".to_string(), wireless_setting);

        if let Some(psk) = psk {
            let mut security = std::collections::HashMap::new();
            security.insert("key-mgmt".to_string(), owned("wpa-psk".to_string()));
            security.insert("psk".to_string(), owned(psk.to_string()));
            settings.insert("802-11-wireless-security".to_string(), security);
        }

        let root = OwnedObjectPath::try_from("/").map_err(|e| e.to_string())?;
        let (_connection, _active) = manager
            .add_and_activate_connection(settings, wifi_path, root)
            .await
            .map_err(|error| format!("connect failed: {error}"))?;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let state = manager.state().await.unwrap_or(0);
            if state == 60 || state == 50 {
                return Ok(());
            }
        }
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), String> {
        let manager = ManagerProxy::new(&self.connection)
            .await
            .map_err(|e| e.to_string())?;
        let active = manager
            .active_connections()
            .await
            .map_err(|e| e.to_string())?;
        for path in active {
            manager
                .deactivate_connection(path)
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// Generate a v4 UUID without pulling another crate into the panel plugin.
fn uuid_v4() -> String {
    use std::io::Read;

    let mut bytes = [0u8; 16];
    let random_ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok();
    if !random_ok {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes.copy_from_slice(&now.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn owned<T>(value: T) -> OwnedVariant
where
    Variant<'static>: From<T>,
{
    OwnedVariant::try_from(Variant::from(value)).expect("owned variant")
}

struct Ui {
    button: gtk::Button,
    label: gtk::Label,
    image: gtk::Image,
    snapshot: RefCell<Snapshot>,
    tx: Sender<Request>,
    busy: Arc<AtomicBool>,
}

impl Ui {
    fn update_label(&self, snapshot: &Snapshot) {
        let (icon, text) = match snapshot.manager_state {
            60 | 50 | 40 => (
                "network-wireless-signal-excellent",
                snapshot
                    .connected_ssid
                    .clone()
                    .map(|ssid| format!("Wi-Fi: {ssid}"))
                    .unwrap_or_else(|| "Wi-Fi: connected".to_string()),
            ),
            30 => (
                "network-wireless-acquiring",
                "Wi-Fi: connecting".to_string(),
            ),
            20 => (
                "network-wireless-signal-none",
                "Wi-Fi: disconnected".to_string(),
            ),
            10 => ("network-wireless-offline", "Wi-Fi: disabled".to_string()),
            _ => ("network-wireless-signal-none", "Wi-Fi".to_string()),
        };
        self.image.set_icon_name(Some(icon));
        self.label.set_text(&text);
        self.button.set_tooltip_text(Some(&text));
    }

    fn rebuild_menu(&self) -> gtk::Menu {
        let menu = gtk::Menu::new();
        let snapshot = self.snapshot.borrow().clone();

        let header = gtk::MenuItem::with_label(match snapshot.manager_state {
            60 | 50 | 40 => "Wi-Fi connected",
            30 => "Connecting…",
            20 => "Wi-Fi networks",
            _ => "Wi-Fi unavailable",
        });
        header.set_sensitive(false);
        menu.append(&header);

        for ap in &snapshot.aps {
            let label = if ap.privacy {
                format!("{}  {}%  locked", ap.label, ap.strength)
            } else {
                format!("{}  {}%", ap.label, ap.strength)
            };
            let item = gtk::MenuItem::with_label(&label);
            let tx = self.tx.clone();
            let ssid = ap.ssid.clone();
            let privacy = ap.privacy;
            let button = self.button.clone();
            item.connect_activate(move |_| {
                let psk = if privacy {
                    prompt_password(&button)
                } else {
                    Some(None)
                };
                if let Some(psk) = psk {
                    let _ = tx.send(Request::Connect {
                        ssid: ssid.clone(),
                        psk,
                    });
                }
            });
            menu.append(&item);
        }

        if snapshot.aps.is_empty() {
            let empty = gtk::MenuItem::with_label("No networks found — rescanning");
            empty.set_sensitive(false);
            menu.append(&empty);
            let _ = self.tx.send(Request::Rescan);
        }

        menu.append(&gtk::SeparatorMenuItem::new());

        let rescan = gtk::MenuItem::with_label("Rescan");
        let tx = self.tx.clone();
        rescan.connect_activate(move |_| {
            let _ = tx.send(Request::Rescan);
        });
        menu.append(&rescan);

        let disconnect = gtk::MenuItem::with_label("Disconnect");
        let tx = self.tx.clone();
        disconnect.connect_activate(move |_| {
            let _ = tx.send(Request::Disconnect);
        });
        menu.append(&disconnect);

        menu.show_all();
        menu
    }
}

fn prompt_password(button: &gtk::Button) -> Option<Option<String>> {
    let parent = button
        .toplevel()
        .and_then(|widget| widget.downcast::<gtk::Window>().ok());
    let dialog = gtk::Dialog::with_buttons(
        Some("Wi-Fi password"),
        parent.as_ref(),
        gtk::DialogFlags::MODAL,
        &[
            ("Cancel", gtk::ResponseType::Cancel),
            ("Connect", gtk::ResponseType::Ok),
        ],
    );
    dialog.set_default_response(gtk::ResponseType::Ok);
    let entry = gtk::Entry::new();
    entry.set_visibility(false);
    entry.set_activates_default(true);
    entry.set_placeholder_text(Some("WPA passphrase"));
    dialog.content_area().add(&entry);
    dialog.show_all();
    let response = dialog.run();
    let text = entry.text().to_string();
    dialog.close();
    if response == gtk::ResponseType::Ok && !text.is_empty() {
        Some(Some(text))
    } else {
        None
    }
}

struct Worker {
    tx: Sender<Request>,
}

impl Worker {
    fn spawn(ui_tx: Sender<UiEvent>) -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        let worker_tx = tx.clone();
        let _ = std::thread::Builder::new()
            .name("nm-xfce-dbus".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let Ok(runtime) = runtime else {
                    let _ = ui_tx.send(UiEvent::Error("failed to start async runtime".into()));
                    return;
                };
                runtime.block_on(async move {
                    let signal_tx = worker_tx.clone();
                    tokio::spawn(signal_loop(signal_tx));
                    worker_loop(rx, ui_tx).await;
                });
            });
        Self { tx }
    }
}

/// Listen for NetworkManager signals and enqueue a coalesced refresh.
///
/// The panel still has a slow fallback poll, but normal state changes
/// (connect/disconnect, AP strength, IP4Config) arrive as D-Bus signals.
async fn signal_loop(tx: Sender<Request>) {
    loop {
        if let Err(error) = listen_signals(&tx).await {
            eprintln!("nm-freebsd-xfce: signal listener: {error}");
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn listen_signals(tx: &Sender<Request>) -> Result<(), String> {
    let connection = ConnectionBuilder::system()
        .build()
        .await
        .map_err(|error| error.to_string())?;
    // Match by path namespace rather than a well-known sender: signals are
    // sent by the daemon's unique bus name, and this also survives a daemon
    // restart with a new unique name.
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .path_namespace("/org/freedesktop/NetworkManager")
        .build()
        .map_err(|error| error.to_string())?;
    let mut stream = MessageStream::for_match_rule(rule, &connection, None)
        .await
        .map_err(|error| error.to_string())?;
    while stream.next().await.is_some() {
        let _ = tx.send(Request::Refresh);
        // Collapse a burst of AP PropertiesChanged signals into one snapshot.
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

async fn worker_loop(rx: Receiver<Request>, ui_tx: Sender<UiEvent>) {
    while let Ok(request) = rx.recv() {
        match request {
            Request::Shutdown => return,
            Request::Refresh => {
                let _ = ui_tx.send(UiEvent::Busy(true));
                match refresh_snapshot().await {
                    Ok(snapshot) => {
                        let _ = ui_tx.send(UiEvent::Snapshot(snapshot));
                    }
                    Err(error) => {
                        let _ = ui_tx.send(UiEvent::Error(error));
                    }
                }
                let _ = ui_tx.send(UiEvent::Busy(false));
            }
            Request::Rescan => {
                let _ = ui_tx.send(UiEvent::Busy(true));
                let result = async {
                    let client = BusClient::connect().await?;
                    client.rescan().await?;
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                    client.snapshot().await
                }
                .await;
                match result {
                    Ok(snapshot) => {
                        let _ = ui_tx.send(UiEvent::Snapshot(snapshot));
                    }
                    Err(error) => {
                        let _ = ui_tx.send(UiEvent::Error(error));
                    }
                }
                let _ = ui_tx.send(UiEvent::Busy(false));
            }
            Request::Connect { ssid, psk } => {
                let _ = ui_tx.send(UiEvent::Busy(true));
                let result = async {
                    let client = BusClient::connect().await?;
                    client.connect_to(&ssid, psk.as_deref()).await?;
                    client.snapshot().await
                }
                .await;
                match result {
                    Ok(snapshot) => {
                        let _ = ui_tx.send(UiEvent::Snapshot(snapshot));
                    }
                    Err(error) => {
                        let _ = ui_tx.send(UiEvent::Error(error));
                    }
                }
                let _ = ui_tx.send(UiEvent::Busy(false));
            }
            Request::Disconnect => {
                let _ = ui_tx.send(UiEvent::Busy(true));
                let result = async {
                    let client = BusClient::connect().await?;
                    client.disconnect().await?;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    client.snapshot().await
                }
                .await;
                match result {
                    Ok(snapshot) => {
                        let _ = ui_tx.send(UiEvent::Snapshot(snapshot));
                    }
                    Err(error) => {
                        let _ = ui_tx.send(UiEvent::Error(error));
                    }
                }
                let _ = ui_tx.send(UiEvent::Busy(false));
            }
        }
    }
}

async fn refresh_snapshot() -> Result<Snapshot, String> {
    let client = BusClient::connect().await?;
    client.snapshot().await
}

#[derive(Default)]
pub struct PluginState {
    ui: Option<Rc<Ui>>,
    worker: Option<Worker>,
    timers: Vec<glib::SourceId>,
}

impl std::fmt::Debug for PluginState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginState").finish_non_exhaustive()
    }
}

pub struct NetworkManagerPlugin;

impl PanelPlugin for NetworkManagerPlugin {
    const TYPE_NAME: &'static str = "XfceNmFreeBsd";
    type State = PluginState;

    fn construct(state: &mut Self::State, plugin: &Plugin) {
        let button = gtk::Button::new();
        button.set_relief(gtk::ReliefStyle::None);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let image =
            gtk::Image::from_icon_name(Some("network-wireless-signal-none"), gtk::IconSize::Menu);
        image.set_pixel_size(16);
        let label = gtk::Label::new(Some("Wi-Fi"));
        row.pack_start(&image, false, false, 0);
        row.pack_start(&label, false, false, 0);
        button.add(&row);
        button.show_all();
        plugin.add_widget(&button);

        let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>();
        let worker = Worker::spawn(ui_tx);
        let ui = Rc::new(Ui {
            button: button.clone(),
            label: label.clone(),
            image: image.clone(),
            snapshot: RefCell::new(Snapshot::default()),
            tx: worker.tx.clone(),
            busy: Arc::new(AtomicBool::new(false)),
        });

        {
            let ui = Rc::downgrade(&ui);
            button.connect_clicked(move |button| {
                let Some(ui) = ui.upgrade() else {
                    return;
                };
                if ui.busy.load(Ordering::SeqCst) {
                    return;
                }
                let menu = ui.rebuild_menu();
                menu.popup_at_widget(
                    button,
                    gtk::gdk::Gravity::South,
                    gtk::gdk::Gravity::North,
                    None,
                );
            });
        }

        {
            let ui = ui.clone();
            let ui_rx = RefCell::new(ui_rx);
            let timer = glib::timeout_add_local(Duration::from_millis(150), move || {
                while let Ok(event) = ui_rx.borrow_mut().try_recv() {
                    match event {
                        UiEvent::Snapshot(snapshot) => {
                            *ui.snapshot.borrow_mut() = snapshot.clone();
                            ui.update_label(&snapshot);
                        }
                        UiEvent::Busy(busy) => {
                            ui.busy.store(busy, Ordering::SeqCst);
                            ui.button.set_sensitive(!busy);
                        }
                        UiEvent::Error(error) => {
                            ui.label.set_text("Wi-Fi unavailable");
                            ui.image.set_icon_name(Some("network-wireless-offline"));
                            ui.button.set_tooltip_text(Some(&error));
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
            state.timers.push(timer);
        }

        {
            let tx = ui.tx.clone();
            // Fallback only; normal updates are signal-driven and immediate.
            let timer = glib::timeout_add_local(Duration::from_secs(30), move || {
                let _ = tx.send(Request::Refresh);
                glib::ControlFlow::Continue
            });
            state.timers.push(timer);
        }

        state.ui = Some(ui);
        state.worker = Some(worker);
        let _ = state.ui.as_ref().map(|ui| ui.tx.send(Request::Refresh));
    }

    fn size_changed(_state: &mut Self::State, plugin: &Plugin, size: i32) -> bool {
        plugin.set_size_request(size, size);
        true
    }

    fn free_data(state: &mut Self::State, _plugin: &Plugin) {
        for timer in state.timers.drain(..) {
            timer.remove();
        }
        if let Some(worker) = state.worker.take() {
            let _ = worker.tx.send(Request::Shutdown);
        }
        state.ui = None;
    }
}

xfce_panel::define_panel_plugin!(NetworkManagerPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_has_v4_shape() {
        let uuid = uuid_v4();
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.as_bytes()[14], b'4');
        assert!(matches!(uuid.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert_eq!(uuid.chars().filter(|c| *c == '-').count(), 4);
    }
}
