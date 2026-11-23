//! Network backends.
//!
//! FreeBSD's base wpa_supplicant is built without CONFIG_CTRL_IFACE_DBUS, so
//! the control socket is the transport that works out of the box. A D-Bus
//! backend is kept for builds/ports where fi.w1.wpa_supplicant1 exists.

pub mod wpa_ctrl;
pub mod wpa_dbus;

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::config::{BackendKind, Config};

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("wpa_supplicant control socket {path}: {source}")]
    ControlSocket {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("wpa_supplicant rejected the command: {0}")]
    Rejected(String),
    #[error("wpa_supplicant D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),
    #[error("wpa_supplicant D-Bus service is not available")]
    DbusUnavailable,
    #[error("invalid wpa_supplicant response: {0}")]
    InvalidResponse(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WpaState {
    Disconnected,
    Inactive,
    InterfaceDisabled,
    Scanning,
    Authenticating,
    Associating,
    Associated,
    FourWayHandshake,
    GroupHandshake,
    Completed,
    #[default]
    Unknown,
}

impl WpaState {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_uppercase().as_str() {
            "DISCONNECTED" => Self::Disconnected,
            "INACTIVE" => Self::Inactive,
            "INTERFACE_DISABLED" => Self::InterfaceDisabled,
            "SCANNING" => Self::Scanning,
            "AUTHENTICATING" => Self::Authenticating,
            "ASSOCIATING" => Self::Associating,
            "ASSOCIATED" => Self::Associated,
            "4WAY_HANDSHAKE" => Self::FourWayHandshake,
            "GROUP_HANDSHAKE" => Self::GroupHandshake,
            "COMPLETED" => Self::Completed,
            _ => Self::Unknown,
        }
    }

    pub fn is_connected(self) -> bool {
        matches!(self, Self::Completed)
    }

    pub fn is_connecting(self) -> bool {
        matches!(
            self,
            Self::Associating
                | Self::Associated
                | Self::FourWayHandshake
                | Self::GroupHandshake
                | Self::Authenticating
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WifiStatus {
    pub state: WpaState,
    pub ssid: Vec<u8>,
    pub bssid: Option<String>,
    pub ip_address: Option<String>,
    pub key_mgmt: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScanAp {
    pub bssid: String,
    pub ssid: Vec<u8>,
    pub frequency: u32,
    pub signal: i32,
    pub flags: String,
    pub security: nm_common::security::ApSecurity,
}

impl ScanAp {
    /// NetworkManager reports strength as 0..=100, wpa_supplicant as dBm.
    pub fn strength(&self) -> u8 {
        if self.signal <= -100 {
            0
        } else if self.signal >= -50 {
            100
        } else {
            ((self.signal + 100) * 2) as u8
        }
    }
}

pub struct ConnectRequest<'a> {
    pub ifname: &'a str,
    pub ssid: &'a [u8],
    pub psk: Option<&'a str>,
    pub key_mgmt: &'a str,
    pub bssid: Option<&'a str>,
    pub id_str: &'a str,
    pub hidden: bool,
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    ScanResults { ifname: String },
    StateChanged { ifname: String, status: WifiStatus },
    Error { ifname: String, message: String },
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn kind(&self) -> &'static str;

    async fn scan(&self, ifname: &str) -> Result<(), BackendError>;

    async fn scan_results(&self, ifname: &str) -> Result<Vec<ScanAp>, BackendError>;

    async fn status(&self, ifname: &str) -> Result<WifiStatus, BackendError>;

    async fn connect(&self, request: &ConnectRequest<'_>) -> Result<(), BackendError>;

    async fn disconnect(&self, ifname: &str) -> Result<(), BackendError>;

    /// Reload a backend that reads an on-disk network configuration.
    async fn reconfigure(&self, _ifname: &str) -> Result<(), BackendError> {
        Ok(())
    }

    /// Spawn long-lived event listeners. Implementations must not block.
    fn spawn_events(self: Arc<Self>, tx: mpsc::Sender<BackendEvent>);
}

/// Build the configured backend, probing the D-Bus transport when asked to
/// auto-select. Auto mode is deliberately conservative: stock FreeBSD fails
/// the probe and lands on the control-socket backend.
pub async fn create(
    config: &Config,
    interfaces: &[String],
) -> Result<Arc<dyn Backend>, BackendError> {
    match config.backend {
        BackendKind::WpaCtrl => Ok(Arc::new(wpa_ctrl::WpaCtrlBackend::new(
            config.wpa_ctrl_dir.clone(),
            interfaces.to_vec(),
        ))),
        BackendKind::WpaDbus => {
            let backend =
                wpa_dbus::WpaDbusBackend::new(interfaces.to_vec(), config.bus_address.as_deref())
                    .await?;
            Ok(Arc::new(backend))
        }
        BackendKind::Auto => {
            if wpa_dbus::service_available(config.bus_address.as_deref()).await {
                match wpa_dbus::WpaDbusBackend::new(
                    interfaces.to_vec(),
                    config.bus_address.as_deref(),
                )
                .await
                {
                    Ok(backend) => {
                        tracing::info!("using wpa_supplicant D-Bus backend");
                        return Ok(Arc::new(backend));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "wpa_supplicant D-Bus backend failed; falling back to control socket");
                    }
                }
            }
            tracing::debug!(
                bus_address = ?config.bus_address,
                "fi.w1.wpa_supplicant1 is not available; auto mode fell back to the control socket"
            );
            tracing::info!(
                dir = %config.wpa_ctrl_dir.display(),
                "using wpa_supplicant control-socket backend"
            );
            Ok(Arc::new(wpa_ctrl::WpaCtrlBackend::new(
                config.wpa_ctrl_dir.clone(),
                interfaces.to_vec(),
            )))
        }
    }
}
