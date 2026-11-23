//! NetworkManager D-Bus compatibility layer for FreeBSD.
//!
//! The crate is a library so the integration tests can start the daemon
//! in-process against a private bus and a fake wpa_supplicant control socket.

pub mod backend;
pub mod config;
pub mod core;
pub mod daemon;
pub mod dbus;
pub mod model;
pub mod netlink;
pub mod settings;
pub mod system;
