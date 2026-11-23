//! Daemon startup, desktop dispatch and shutdown.

use std::sync::Arc;

use anyhow::{Context, Result};
use nm_common::DesktopEnvironment;
use nm_common::names;
use tokio::signal::unix::{SignalKind, signal};
use zbus::Connection;
use zbus::connection::Builder;

use crate::backend;
use crate::config::{BusKind, Config};
use crate::core::Core;
use crate::system;

pub struct Running {
    pub core: Arc<Core>,
    /// Kept alive for the process lifetime; dropping it disconnects.
    pub connection: Connection,
}

/// Connect to the bus, own org.freedesktop.NetworkManager, register the object
/// tree and start the backend pumps. Returns once the service is live.
pub async fn start(config: Config) -> Result<Running> {
    let wifi_interfaces = system::discover_wifi_interfaces(&config.wifi_interfaces);
    if wifi_interfaces.is_empty() {
        tracing::warn!(
            "no Wi-Fi interfaces detected; the daemon will still expose wired interfaces"
        );
    } else {
        tracing::info!(?wifi_interfaces, "Wi-Fi interfaces");
    }

    let backend = backend::create(&config, &wifi_interfaces)
        .await
        .context("failed to initialize the wpa_supplicant backend")?;

    let builder = if let Some(address) = &config.bus_address {
        Builder::address(address.as_str())
    } else {
        match config.bus {
            BusKind::System => Builder::system(),
            BusKind::Session => Builder::session(),
        }
    };
    let connection = builder
        .name(names::NM_BUS_NAME)
        .build()
        .await
        .with_context(|| {
            format!(
                "failed to own {} on the {} bus",
                names::NM_BUS_NAME,
                match config.bus {
                    BusKind::System => "system",
                    BusKind::Session => "session",
                }
            )
        })?;

    let core = Core::new(config.clone(), backend);
    core.set_bus(connection.clone(), connection.object_server().clone());
    core.start()
        .await
        .context("failed to register NetworkManager objects")?;

    log_desktop(config.desktop);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        backend = core.backend.kind(),
        "NetworkManager compatibility service is ready"
    );

    Ok(Running { core, connection })
}

/// Run until SIGINT/SIGTERM. Network state is intentionally left alone on
/// shutdown: stopping the compatibility daemon must not drop the user's link.
pub async fn run(config: Config) -> Result<()> {
    let _running = start(config).await?;

    let mut sigterm =
        signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.context("failed to wait for SIGINT")?;
            tracing::info!("SIGINT received, shutting down");
        }
        _ = sigterm.recv() => {
            tracing::info!("SIGTERM received, shutting down");
        }
    }
    Ok(())
}

fn log_desktop(desktop: DesktopEnvironment) {
    match desktop {
        DesktopEnvironment::Kde => tracing::info!(
            "KDE session detected: plasma-nm will use the D-Bus service; the daemon starts no UI"
        ),
        DesktopEnvironment::Gnome => tracing::info!(
            "GNOME session detected: the GNOME network panel will use the D-Bus service; the daemon starts no UI"
        ),
        DesktopEnvironment::Xfce => tracing::info!(
            "XFCE session detected: the nm-freebsd-xfce panel plugin talks to this service; no UI is started by the daemon"
        ),
        DesktopEnvironment::Unknown => tracing::info!(
            "unknown desktop: providing D-Bus only (set XDG_CURRENT_DESKTOP or use --desktop to override)"
        ),
    }
}
