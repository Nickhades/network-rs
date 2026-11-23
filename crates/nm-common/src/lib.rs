//! Shared vocabulary for the NetworkManager-on-FreeBSD compatibility layer.
//!
//! This crate is deliberately dependency-free: both the system daemon and the
//! XFCE panel plugin depend on it, and the plugin should stay as small as
//! possible.

pub mod consts;
pub mod desktop;
pub mod names;
pub mod security;
pub mod ssid;

pub use desktop::DesktopEnvironment;
