//! Daemon configuration.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use nm_common::DesktopEnvironment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendKind {
    /// Probe wpa_supplicant D-Bus first, then fall back to its control socket.
    Auto,
    /// Use wpa_supplicant's Unix control socket (works on stock FreeBSD).
    WpaCtrl,
    /// Use the fi.w1.wpa_supplicant1 D-Bus service.
    WpaDbus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BusKind {
    System,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DesktopChoice {
    Auto,
    Kde,
    Gnome,
    Xfce,
    Unknown,
}

impl From<DesktopChoice> for Option<DesktopEnvironment> {
    fn from(value: DesktopChoice) -> Self {
        match value {
            DesktopChoice::Auto => None,
            DesktopChoice::Kde => Some(DesktopEnvironment::Kde),
            DesktopChoice::Gnome => Some(DesktopEnvironment::Gnome),
            DesktopChoice::Xfce => Some(DesktopEnvironment::Xfce),
            DesktopChoice::Unknown => Some(DesktopEnvironment::Unknown),
        }
    }
}

#[derive(Debug, Clone, Parser)]
#[command(
    name = "nm-freebsd-daemon",
    version,
    about = "NetworkManager D-Bus compatibility layer for FreeBSD"
)]
pub struct Args {
    /// Which wpa_supplicant transport to use.
    #[arg(long, value_enum, default_value_t = BackendKind::Auto)]
    pub backend: BackendKind,

    /// Directory holding wpa_supplicant per-interface control sockets.
    #[arg(long, default_value = "/var/run/wpa_supplicant")]
    pub wpa_ctrl_dir: PathBuf,

    /// Directory for saved connection profiles.
    #[arg(long, default_value = "/var/db/nm-freebsd")]
    pub state_dir: PathBuf,

    /// Wi-Fi interface to manage. May be repeated; auto-detected when omitted.
    #[arg(long = "wifi-interface")]
    pub wifi_interfaces: Vec<String>,

    /// Do not expose wired/other interfaces.
    #[arg(long)]
    pub no_wired: bool,

    /// Bus to connect to. The system bus is the production setting.
    #[arg(long, value_enum, default_value_t = BusKind::System)]
    pub bus: BusKind,

    /// Explicit D-Bus address (for jails, tests and nonstandard bus paths).
    #[arg(long)]
    pub bus_address: Option<String>,

    /// Override desktop detection (useful for testing and for headless images).
    #[arg(long, value_enum, default_value_t = DesktopChoice::Auto)]
    pub desktop: DesktopChoice,

    /// Skip the initial Wi-Fi scan at startup.
    #[arg(long)]
    pub no_initial_scan: bool,

    /// Verbose log filter, for example "debug" or "nm_freebsd_daemon=trace".
    #[arg(long, env = "NM_FREEBSD_LOG")]
    pub log_level: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub backend: BackendKind,
    pub wpa_ctrl_dir: PathBuf,
    pub state_dir: PathBuf,
    pub wifi_interfaces: Vec<String>,
    pub include_wired: bool,
    pub bus: BusKind,
    pub bus_address: Option<String>,
    pub desktop: DesktopEnvironment,
    pub initial_scan: bool,
    pub log_level: String,
}

impl Config {
    pub fn from_args(args: Args) -> Self {
        Self {
            backend: args.backend,
            wpa_ctrl_dir: args.wpa_ctrl_dir,
            state_dir: args.state_dir,
            wifi_interfaces: args.wifi_interfaces,
            include_wired: !args.no_wired,
            bus: args.bus,
            bus_address: args.bus_address,
            desktop: Option::<DesktopEnvironment>::from(args.desktop)
                .unwrap_or_else(DesktopEnvironment::detect),
            initial_scan: !args.no_initial_scan,
            log_level: args.log_level.unwrap_or_else(|| "info".to_string()),
        }
    }

    pub fn for_tests(wpa_ctrl_dir: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            backend: BackendKind::WpaCtrl,
            wpa_ctrl_dir,
            state_dir,
            wifi_interfaces: vec!["wlan0".to_string()],
            include_wired: false,
            bus: BusKind::System,
            bus_address: None,
            desktop: DesktopEnvironment::Xfce,
            initial_scan: true,
            log_level: "debug".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_backend_names() {
        assert_eq!(BackendKind::WpaCtrl, BackendKind::WpaCtrl);
        let args = Args::try_parse_from([
            "nm-freebsd-daemon",
            "--backend",
            "wpa-ctrl",
            "--wifi-interface",
            "wlan0",
            "--wifi-interface",
            "wlan1",
        ])
        .unwrap();
        let cfg = Config::from_args(args);
        assert_eq!(cfg.backend, BackendKind::WpaCtrl);
        assert_eq!(cfg.wifi_interfaces, vec!["wlan0", "wlan1"]);
    }
}
