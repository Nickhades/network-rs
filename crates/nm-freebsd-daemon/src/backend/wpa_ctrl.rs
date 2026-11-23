//! wpa_supplicant control-socket client.
//!
//! FreeBSD's wpa_supplicant control interface is a Unix **datagram** socket.
//! Each client binds a private path under /tmp, connects to the interface
//! socket, sends one command and receives one reply datagram. Events are
//! delivered on a long-lived socket after ATTACH.
//!
//! Stock FreeBSD builds wpa_supplicant without CONFIG_CTRL_IFACE_DBUS, so this
//! is the transport that works out of the box.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nm_common::security::parse_scan_flags;
use nm_common::ssid::{escape_wpa_string, hex_encode, unescape_wpa};
use tokio::net::UnixDatagram;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::{Backend, BackendError, BackendEvent, ConnectRequest, ScanAp, WifiStatus, WpaState};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(4);
const EVENT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

static CLIENT_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct WpaCtrlBackend {
    dir: PathBuf,
    interfaces: Vec<String>,
}

/// A bound datagram socket plus the path that must be removed on drop.
struct ClientSocket {
    socket: UnixDatagram,
    path: PathBuf,
}

impl ClientSocket {
    async fn connect(server: &Path) -> Result<Self, BackendError> {
        let seq = CLIENT_SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nm-freebsd-{}-{seq}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let socket = UnixDatagram::bind(&path).map_err(|source| BackendError::ControlSocket {
            path: path.display().to_string(),
            source,
        })?;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        socket
            .connect(server)
            .map_err(|source| BackendError::ControlSocket {
                path: server.display().to_string(),
                source,
            })?;
        Ok(Self { socket, path })
    }

    async fn send(&self, command: &str) -> Result<(), BackendError> {
        self.socket
            .send(command.as_bytes())
            .await
            .map(|_| ())
            .map_err(|source| BackendError::ControlSocket {
                path: self.path.display().to_string(),
                source,
            })
    }

    async fn recv(&self, wait: Duration) -> Result<Vec<u8>, BackendError> {
        let mut buf = vec![0u8; 65_536];
        let n = timeout(wait, self.socket.recv(&mut buf))
            .await
            .map_err(|_| BackendError::ControlSocket {
                path: self.path.display().to_string(),
                source: std::io::Error::new(std::io::ErrorKind::TimedOut, "reply timed out"),
            })?
            .map_err(|source| BackendError::ControlSocket {
                path: self.path.display().to_string(),
                source,
            })?;
        buf.truncate(n);
        Ok(buf)
    }
}

impl Drop for ClientSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl WpaCtrlBackend {
    pub fn new(dir: PathBuf, interfaces: Vec<String>) -> Self {
        Self { dir, interfaces }
    }

    fn socket_path(&self, ifname: &str) -> PathBuf {
        self.dir.join(ifname)
    }

    async fn command(&self, ifname: &str, command: &str) -> Result<Vec<u8>, BackendError> {
        let server = self.socket_path(ifname);
        let client = ClientSocket::connect(&server).await?;
        client.send(command).await?;

        loop {
            let response = client.recv(COMMAND_TIMEOUT).await?;
            if response.first() == Some(&b'<') {
                // An asynchronous event raced ahead of the command reply.
                continue;
            }
            let text = String::from_utf8_lossy(&response);
            if let Some(fail) = text.lines().find(|line| line.starts_with("FAIL")) {
                return Err(BackendError::Rejected(fail.to_string()));
            }
            return Ok(response);
        }
    }
}

pub fn parse_scan_results(raw: &[u8]) -> Result<Vec<ScanAp>, BackendError> {
    let mut out = Vec::new();
    let mut lines = raw.split(|b| *b == b'\n');
    let _header = lines.next();
    for line in lines {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&[u8]> = line.splitn(5, |b| *b == b'\t').collect();
        if fields.len() < 5 {
            continue;
        }
        let bssid = String::from_utf8_lossy(fields[0])
            .trim()
            .to_ascii_lowercase();
        let frequency = String::from_utf8_lossy(fields[1])
            .trim()
            .parse::<u32>()
            .unwrap_or(0);
        let signal = String::from_utf8_lossy(fields[2])
            .trim()
            .parse::<i32>()
            .unwrap_or(-100);
        let flags = String::from_utf8_lossy(fields[3]).trim().to_string();
        let ssid = unescape_wpa(fields[4]);
        let security = parse_scan_flags(&flags);
        out.push(ScanAp {
            bssid,
            ssid,
            frequency,
            signal,
            flags,
            security,
        });
    }
    Ok(out)
}

pub fn parse_status(raw: &[u8]) -> WifiStatus {
    let text = String::from_utf8_lossy(raw);
    let mut status = WifiStatus::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "wpa_state" => status.state = WpaState::parse(value),
            "ssid" => status.ssid = unescape_wpa(value.as_bytes()),
            "bssid" => status.bssid = Some(value.trim().to_ascii_lowercase()),
            "ip_address" => status.ip_address = Some(value.trim().to_string()),
            "key_mgmt" => status.key_mgmt = Some(value.trim().to_string()),
            _ => {}
        }
    }
    status
}

#[async_trait]
impl Backend for WpaCtrlBackend {
    fn kind(&self) -> &'static str {
        "wpa_ctrl"
    }

    async fn scan(&self, ifname: &str) -> Result<(), BackendError> {
        match self.command(ifname, "SCAN").await {
            Ok(_) => Ok(()),
            Err(BackendError::Rejected(message)) if message.contains("FAIL-BUSY") => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn scan_results(&self, ifname: &str) -> Result<Vec<ScanAp>, BackendError> {
        let raw = self.command(ifname, "SCAN_RESULTS").await?;
        parse_scan_results(&raw)
    }

    async fn status(&self, ifname: &str) -> Result<WifiStatus, BackendError> {
        let raw = self.command(ifname, "STATUS").await?;
        Ok(parse_status(&raw))
    }

    async fn connect(&self, request: &ConnectRequest<'_>) -> Result<(), BackendError> {
        if request.key_mgmt == "WPA-EAP" {
            return Err(BackendError::Other(
                "enterprise (802.1X) authentication needs a secret agent, which this MVP does not implement"
                    .to_string(),
            ));
        }
        if request.key_mgmt == "WEP" {
            return Err(BackendError::Other(
                "WEP profiles are not supported".to_string(),
            ));
        }

        let id = String::from_utf8_lossy(&self.command(request.ifname, "ADD_NETWORK").await?)
            .trim()
            .to_string();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(BackendError::InvalidResponse(format!(
                "ADD_NETWORK returned {id:?}"
            )));
        }

        let ssid_hex = hex_encode(request.ssid);
        self.command(request.ifname, &format!("SET_NETWORK {id} ssid {ssid_hex}"))
            .await?;

        if request.key_mgmt == "NONE" {
            self.command(request.ifname, &format!("SET_NETWORK {id} key_mgmt NONE"))
                .await?;
        } else {
            if let Some(psk) = request.psk {
                let quoted = escape_wpa_string(psk);
                self.command(request.ifname, &format!("SET_NETWORK {id} psk {quoted}"))
                    .await?;
            }
            self.command(
                request.ifname,
                &format!("SET_NETWORK {id} key_mgmt {}", request.key_mgmt),
            )
            .await?;
            if request.key_mgmt == "SAE" {
                self.command(request.ifname, &format!("SET_NETWORK {id} ieee80211w 2"))
                    .await?;
            }
        }

        if request.hidden {
            self.command(request.ifname, &format!("SET_NETWORK {id} scan_ssid 1"))
                .await?;
        }
        self.command(request.ifname, &format!("ENABLE_NETWORK {id}"))
            .await?;
        self.command(request.ifname, &format!("SELECT_NETWORK {id}"))
            .await?;
        // Deliberately no SAVE_CONFIG: the daemon owns profiles in its own
        // state directory and must not rewrite /etc/wpa_supplicant.conf.
        Ok(())
    }

    async fn disconnect(&self, ifname: &str) -> Result<(), BackendError> {
        self.command(ifname, "DISCONNECT").await?;
        Ok(())
    }

    fn spawn_events(self: Arc<Self>, tx: mpsc::Sender<BackendEvent>) {
        for ifname in self.interfaces.clone() {
            let backend = Arc::clone(&self);
            let tx = tx.clone();
            tokio::spawn(async move {
                backend.event_loop(ifname, tx).await;
            });
        }
    }
}

impl WpaCtrlBackend {
    async fn event_loop(&self, ifname: String, tx: mpsc::Sender<BackendEvent>) {
        loop {
            let server = self.socket_path(&ifname);
            let client = match ClientSocket::connect(&server).await {
                Ok(client) => client,
                Err(error) => {
                    tracing::debug!(ifname, path = %server.display(), %error, "wpa_supplicant control socket unavailable");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            if client.send("ATTACH").await.is_err() {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            tracing::debug!(ifname, "attached to wpa_supplicant events");
            loop {
                match client.recv(EVENT_IDLE_TIMEOUT).await {
                    Ok(response) => {
                        let text = String::from_utf8_lossy(&response);
                        for line in text.lines() {
                            self.handle_event_line(&ifname, line, &tx).await;
                        }
                    }
                    Err(error) => {
                        tracing::debug!(ifname, %error, "wpa_supplicant event stream ended");
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn handle_event_line(&self, ifname: &str, line: &str, tx: &mpsc::Sender<BackendEvent>) {
        let event = line
            .strip_prefix('<')
            .and_then(|rest| rest.split_once('>'))
            .map(|(_, event)| event)
            .unwrap_or(line);
        if event.starts_with("CTRL-EVENT-SCAN-RESULTS") {
            let _ = tx
                .send(BackendEvent::ScanResults {
                    ifname: ifname.to_string(),
                })
                .await;
        } else if event.starts_with("CTRL-EVENT-CONNECTED")
            || event.starts_with("CTRL-EVENT-DISCONNECTED")
            || event.starts_with("CTRL-EVENT-SSID-")
            || event.starts_with("CTRL-EVENT-ASSOC-REJECT")
            || event.starts_with("CTRL-EVENT-AUTH-REJECT")
        {
            match self.status(ifname).await {
                Ok(status) => {
                    let _ = tx
                        .send(BackendEvent::StateChanged {
                            ifname: ifname.to_string(),
                            status,
                        })
                        .await;
                }
                Err(error) => {
                    let _ = tx
                        .send(BackendEvent::Error {
                            ifname: ifname.to_string(),
                            message: error.to_string(),
                        })
                        .await;
                }
            }
        }
    }
}

/// Read a single command reply. Exposed for manual diagnostics.
pub async fn command_once(socket: &Path, command: &str) -> Result<String, BackendError> {
    let backend = WpaCtrlBackend::new(
        socket
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        Vec::new(),
    );
    let ifname = socket
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let raw = backend.command(&ifname, command).await?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCAN: &[u8] = b"bssid / frequency / signal level / flags / ssid\n\
aa:bb:cc:dd:ee:01\t2412\t-42\t[WPA2-PSK-CCMP][ESS]\tCafe WiFi\n\
aa:bb:cc:dd:ee:02\t5180\t-77\t[ESS]\tOpen Net\n";

    #[test]
    fn parses_scan_results_including_spaces() {
        let aps = parse_scan_results(SCAN).unwrap();
        assert_eq!(aps.len(), 2);
        assert_eq!(aps[0].ssid, b"Cafe WiFi");
        assert_eq!(aps[0].strength(), 100);
        assert!(aps[0].security.has_psk());
        assert_eq!(aps[1].frequency, 5180);
        assert!(aps[1].security.is_open());
        assert_eq!(aps[1].strength(), 46);
    }

    #[test]
    fn decodes_wpa_escaped_ssid() {
        let raw = b"bssid / frequency / signal level / flags / ssid\n\
aa:bb:cc:dd:ee:01\t2412\t-42\t[ESS]\t\\xe4\\xba\\x91\\xe6\\xb5\\xb7\n";
        let aps = parse_scan_results(raw).unwrap();
        assert_eq!(aps[0].ssid, "云海".as_bytes());
    }

    #[test]
    fn parses_status() {
        let raw = b"bssid=aa:bb:cc:dd:ee:01\nfreq=2412\nssid=Cafe WiFi\nid=0\nmode=station\nwpa_state=COMPLETED\nip_address=192.0.2.10\nkey_mgmt=WPA2-PSK\n";
        let status = parse_status(raw);
        assert_eq!(status.state, WpaState::Completed);
        assert!(status.state.is_connected());
        assert_eq!(status.ssid, b"Cafe WiFi");
        assert_eq!(status.bssid.as_deref(), Some("aa:bb:cc:dd:ee:01"));
        assert_eq!(status.ip_address.as_deref(), Some("192.0.2.10"));
    }

    #[test]
    fn strength_clamps() {
        let ap = ScanAp {
            bssid: "x".into(),
            ssid: vec![],
            frequency: 2412,
            signal: -130,
            flags: String::new(),
            security: Default::default(),
        };
        assert_eq!(ap.strength(), 0);
    }
}
