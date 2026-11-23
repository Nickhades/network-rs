//! wpa_supplicant D-Bus backend (fi.w1.wpa_supplicant1).
//!
//! Base-system FreeBSD wpa_supplicant is compiled without this interface;
//! the backend exists for ports builds and for Linux-side development.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use nm_common::consts::ap_security_flags as sec;
use nm_common::security::ApSecurity;
use tokio::sync::mpsc;
use tokio::time::timeout;
use zbus::{Connection, OwnedObjectPath, OwnedVariant, Variant, proxy};

use super::{Backend, BackendError, BackendEvent, ConnectRequest, ScanAp, WifiStatus, WpaState};

#[proxy(
    interface = "fi.w1.wpa_supplicant1",
    default_service = "fi.w1.wpa_supplicant1",
    default_path = "/fi/w1/wpa_supplicant1"
)]
trait WpaSupplicant {
    fn create_interface(
        &self,
        args: HashMap<String, OwnedVariant>,
    ) -> zbus::Result<OwnedObjectPath>;
    fn get_interface(&self, ifname: &str) -> zbus::Result<OwnedObjectPath>;
    fn remove_interface(&self, iface: &OwnedObjectPath) -> zbus::Result<()>;
    #[zbus(property)]
    fn interfaces(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[proxy(
    interface = "fi.w1.wpa_supplicant1.Interface",
    default_service = "fi.w1.wpa_supplicant1",
    default_path = "/fi/w1/wpa_supplicant1/Interfaces/0"
)]
trait WpaInterface {
    fn scan(&self, args: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
    fn add_network(&self, args: HashMap<String, OwnedVariant>) -> zbus::Result<OwnedObjectPath>;
    fn select_network(&self, network: &OwnedObjectPath) -> zbus::Result<()>;
    fn remove_network(&self, network: &OwnedObjectPath) -> zbus::Result<()>;
    fn disconnect(&self) -> zbus::Result<()>;
    fn reconnect(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn ifname(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
    #[zbus(property, name = "BSSs")]
    fn bsss(&self) -> zbus::Result<HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>>>;
    #[zbus(property, name = "CurrentBSS")]
    fn current_bss(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(signal)]
    fn scan_done(&self, success: bool) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.DBus",
    default_service = "org.freedesktop.DBus",
    default_path = "/org/freedesktop/DBus"
)]
trait DBus {
    fn name_has_owner(&self, name: &str) -> zbus::Result<bool>;
}

pub struct WpaDbusBackend {
    connection: Connection,
    interfaces: Vec<String>,
    paths: HashMap<String, OwnedObjectPath>,
}

/// Probe for the wpa_supplicant D-Bus service without making it fatal when no
/// system bus is reachable (the daemon may be started in a container or test).
async fn open_connection(address: Option<&str>) -> Result<Connection, BackendError> {
    match address {
        Some(address) => zbus::connection::Builder::address(address)
            .build()
            .await
            .map_err(BackendError::from),
        None => Connection::system().await.map_err(BackendError::from),
    }
}

pub async fn service_available(address: Option<&str>) -> bool {
    let Ok(conn) = timeout(Duration::from_secs(1), open_connection(address)).await else {
        return false;
    };
    let Ok(conn) = conn else {
        return false;
    };
    let Ok(proxy) = DBusProxy::new(&conn).await else {
        return false;
    };
    matches!(
        timeout(
            Duration::from_secs(1),
            proxy.name_has_owner("fi.w1.wpa_supplicant1")
        )
        .await,
        Ok(Ok(true))
    )
}

impl WpaDbusBackend {
    pub async fn new(interfaces: Vec<String>, address: Option<&str>) -> Result<Self, BackendError> {
        let connection = open_connection(address).await?;
        let supp = WpaSupplicantProxy::new(&connection).await?;
        let mut paths = HashMap::new();
        for ifname in &interfaces {
            let path = match supp.get_interface(ifname).await {
                Ok(path) => path,
                Err(_) => {
                    let mut args = HashMap::new();
                    args.insert("Ifname".to_string(), owned(ifname.to_string()));
                    supp.create_interface(args).await?
                }
            };
            paths.insert(ifname.clone(), path);
        }
        Ok(Self {
            connection,
            interfaces,
            paths,
        })
    }

    async fn iface(&self, ifname: &str) -> Result<WpaInterfaceProxy<'_>, BackendError> {
        let path = self
            .paths
            .get(ifname)
            .ok_or_else(|| BackendError::DbusUnavailable)?;
        Ok(WpaInterfaceProxy::builder(&self.connection)
            .path(path.clone())
            .build()
            .await?)
    }

    async fn bss_map(
        &self,
        ifname: &str,
    ) -> Result<HashMap<OwnedObjectPath, HashMap<String, OwnedVariant>>, BackendError> {
        Ok(self.iface(ifname).await?.bsss().await?)
    }
}

fn owned<T>(value: T) -> OwnedVariant
where
    Variant<'static>: From<T>,
{
    OwnedVariant::try_from(Variant::from(value)).expect("owned variant")
}

fn clone_variant(value: &OwnedVariant) -> Option<OwnedVariant> {
    value.try_clone().ok()
}

fn bssid_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn get_vec_u8(props: &HashMap<String, OwnedVariant>, key: &str) -> Option<Vec<u8>> {
    props
        .get(key)
        .and_then(clone_variant)
        .and_then(|v| Vec::<u8>::try_from(v).ok())
}

fn get_string(props: &HashMap<String, OwnedVariant>, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(clone_variant)
        .and_then(|v| String::try_from(v).ok())
}

fn get_u32(props: &HashMap<String, OwnedVariant>, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| {
        u32::try_from(v)
            .ok()
            .or_else(|| u16::try_from(v).ok().map(u32::from))
    })
}

fn get_i32(props: &HashMap<String, OwnedVariant>, key: &str) -> Option<i32> {
    props.get(key).and_then(|v| {
        i32::try_from(v)
            .ok()
            .or_else(|| i16::try_from(v).ok().map(i32::from))
    })
}

fn security_from_bss(props: &HashMap<String, OwnedVariant>) -> ApSecurity {
    let privacy = props
        .get("Privacy")
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    let mut out = ApSecurity {
        privacy,
        ..Default::default()
    };
    for (key, target) in [("WPA", &mut out.wpa_flags), ("RSN", &mut out.rsn_flags)] {
        let Some(dict) = props
            .get(key)
            .and_then(clone_variant)
            .and_then(|v| HashMap::<String, OwnedVariant>::try_from(v).ok())
        else {
            continue;
        };
        out.privacy = true;
        apply_security_dict(&dict, target);
    }
    if out.wpa_flags != 0 || out.rsn_flags != 0 {
        out.privacy = true;
    }
    out
}

fn apply_security_dict(dict: &HashMap<String, OwnedVariant>, flags: &mut u32) {
    for (key, bit) in [
        ("ccmp", sec::PAIR_CCMP),
        ("tkip", sec::PAIR_TKIP),
        ("wep40", sec::PAIR_WEP40),
        ("wep104", sec::PAIR_WEP104),
    ] {
        if has_string(dict, "Pairwise", key) {
            *flags |= bit;
        }
        if has_string(dict, "Group", key) {
            *flags |= bit << 4;
        }
    }
    if has_string(dict, "KeyMgmt", "wpa-psk") || has_string(dict, "KeyMgmt", "psk") {
        *flags |= sec::KEY_MGMT_PSK;
    }
    if has_string(dict, "KeyMgmt", "wpa-eap") || has_string(dict, "KeyMgmt", "eap") {
        *flags |= sec::KEY_MGMT_802_1X;
    }
    if has_string(dict, "KeyMgmt", "sae") {
        *flags |= sec::KEY_MGMT_SAE;
    }
    if has_string(dict, "KeyMgmt", "owe") {
        *flags |= sec::KEY_MGMT_OWE;
    }
}

fn has_string(dict: &HashMap<String, OwnedVariant>, key: &str, needle: &str) -> bool {
    dict.get(key)
        .and_then(clone_variant)
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .map(|values| values.iter().any(|v| v.eq_ignore_ascii_case(needle)))
        .unwrap_or(false)
}

fn bss_to_ap(props: &HashMap<String, OwnedVariant>) -> Option<ScanAp> {
    let ssid = get_vec_u8(props, "SSID").unwrap_or_default();
    let bssid_raw = get_vec_u8(props, "BSSID");
    if bssid_raw.is_none() {
        tracing::debug!(
            keys = ?props.keys().collect::<Vec<_>>(),
            ssid = ?ssid,
            "wpa BSS has no convertible BSSID"
        );
    }
    let bssid = bssid_raw
        .map(|bytes| bssid_string(&bytes))
        .or_else(|| get_string(props, "BSSID"))?;
    let frequency = get_u32(props, "Frequency").unwrap_or(0);
    let signal = get_i32(props, "Signal").unwrap_or(-100);
    let security = security_from_bss(props);
    Some(ScanAp {
        bssid,
        ssid,
        frequency,
        signal,
        flags: String::new(),
        security,
    })
}

#[async_trait]
impl Backend for WpaDbusBackend {
    fn kind(&self) -> &'static str {
        "wpa_dbus"
    }

    async fn scan(&self, ifname: &str) -> Result<(), BackendError> {
        let iface = self.iface(ifname).await?;
        iface.scan(HashMap::new()).await?;
        Ok(())
    }

    async fn scan_results(&self, ifname: &str) -> Result<Vec<ScanAp>, BackendError> {
        let map = self.bss_map(ifname).await?;
        tracing::debug!(ifname, bss_count = map.len(), "read wpa D-Bus BSSs");
        let mut out: Vec<ScanAp> = map.values().filter_map(bss_to_ap).collect();
        if out.is_empty() && !map.is_empty() {
            tracing::warn!(
                ifname,
                bss_count = map.len(),
                "BSS entries were not convertible"
            );
        }
        out.sort_by_key(|ap| std::cmp::Reverse(ap.signal));
        Ok(out)
    }

    async fn status(&self, ifname: &str) -> Result<WifiStatus, BackendError> {
        let iface = self.iface(ifname).await?;
        let state = iface.state().await?;
        let mut status = WifiStatus {
            state: WpaState::parse(&state),
            ..Default::default()
        };
        if let Ok(current) = iface.current_bss().await
            && current.as_str() != "/"
            && let Ok(map) = iface.bsss().await
            && let Some(props) = map.get(&current)
        {
            status.ssid = get_vec_u8(props, "SSID").unwrap_or_default();
            status.bssid = get_vec_u8(props, "BSSID").map(|bytes| bssid_string(&bytes));
        }
        status.ip_address = crate::system::ipv4_addresses(ifname)
            .first()
            .map(|addr| addr.address.to_string());
        Ok(status)
    }

    async fn connect(&self, request: &ConnectRequest<'_>) -> Result<(), BackendError> {
        if request.key_mgmt == "WPA-EAP" {
            return Err(BackendError::Other(
                "enterprise (802.1X) authentication is not implemented".to_string(),
            ));
        }
        if request.key_mgmt == "WEP" {
            return Err(BackendError::Other("WEP is not supported".to_string()));
        }
        let iface = self.iface(request.ifname).await?;
        let mut args = HashMap::new();
        args.insert("SSID".to_string(), owned(request.ssid.to_vec()));
        args.insert("KeyMgmt".to_string(), owned(request.key_mgmt.to_string()));
        if let Some(psk) = request.psk {
            args.insert("PSK".to_string(), owned(psk.to_string()));
        }
        if let Some(bssid) = request.bssid.filter(|value| !value.is_empty()) {
            args.insert("BSSID".to_string(), owned(bssid.to_string()));
        }
        if !request.id_str.is_empty() {
            args.insert("id_str".to_string(), owned(request.id_str.to_string()));
        }
        if request.hidden {
            args.insert("ScanSSID".to_string(), owned(true));
        }
        let network = iface.add_network(args).await?;
        iface.select_network(&network).await?;
        Ok(())
    }

    async fn disconnect(&self, ifname: &str) -> Result<(), BackendError> {
        let iface = self.iface(ifname).await?;
        iface.disconnect().await?;
        Ok(())
    }

    fn spawn_events(self: Arc<Self>, tx: mpsc::Sender<BackendEvent>) {
        for ifname in self.interfaces.clone() {
            // State polling keeps the device status fresh even when a
            // wpa_supplicant build is sparse about state signals.
            let backend = Arc::clone(&self);
            let tx_status = tx.clone();
            let status_ifname = ifname.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(2));
                let mut last_status: Option<WifiStatus> = None;
                let mut tick: u64 = 0;
                loop {
                    ticker.tick().await;
                    tick += 1;
                    match backend.status(&status_ifname).await {
                        Ok(status) => {
                            if last_status.as_ref() != Some(&status) {
                                last_status = Some(status.clone());
                                let _ = tx_status
                                    .send(BackendEvent::StateChanged {
                                        ifname: status_ifname.clone(),
                                        status,
                                    })
                                    .await;
                            }
                        }
                        Err(error) => {
                            tracing::debug!(
                                ifname = %status_ifname,
                                %error,
                                "wpa D-Bus status poll failed"
                            );
                        }
                    }
                    // Slow fallback in case a ScanDone was emitted before
                    // this listener attached.
                    if tick.is_multiple_of(5)
                        && let Ok(aps) = backend.scan_results(&status_ifname).await
                        && !aps.is_empty()
                    {
                        let _ = tx_status
                            .send(BackendEvent::ScanResults {
                                ifname: status_ifname.clone(),
                            })
                            .await;
                    }
                }
            });

            // The real D-Bus scan contract: Interface.Scan starts the scan
            // and Interface.ScanDone announces completion. There is no
            // GetScanResults method; results are read from the BSSs property.
            let backend = Arc::clone(&self);
            let tx_scan = tx.clone();
            let scan_ifname = ifname.clone();
            tokio::spawn(async move {
                loop {
                    let iface = match backend.iface(&scan_ifname).await {
                        Ok(iface) => iface,
                        Err(error) => {
                            tracing::debug!(
                                ifname = %scan_ifname,
                                %error,
                                "wpa D-Bus scan signal listener unavailable"
                            );
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                    };
                    let mut signals = match iface.receive_scan_done().await {
                        Ok(signals) => signals,
                        Err(error) => {
                            tracing::debug!(
                                ifname = %scan_ifname,
                                %error,
                                "wpa D-Bus ScanDone subscription failed"
                            );
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                    };
                    while let Some(signal) = signals.next().await {
                        match signal.args() {
                            Ok(args) if *args.success() => {
                                tracing::debug!(
                                    ifname = %scan_ifname,
                                    "wpa D-Bus ScanDone received; refreshing BSSs"
                                );
                                let _ = tx_scan
                                    .send(BackendEvent::ScanResults {
                                        ifname: scan_ifname.clone(),
                                    })
                                    .await;
                            }
                            Ok(_) => {
                                tracing::debug!(
                                    ifname = %scan_ifname,
                                    "wpa_supplicant reported a failed scan"
                                );
                            }
                            Err(error) => {
                                tracing::debug!(
                                    ifname = %scan_ifname,
                                    %error,
                                    "invalid ScanDone signal"
                                );
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            });
        }
    }
}
