//! Small root-run bridge from /etc/dhclient-exit-hooks to nm-freebsd.
//!
//! dhclient exports the lease as environment variables. This tool forwards the
//! subset the daemon needs through the internal Dhcp.Notify D-Bus method. It
//! always exits successfully: a missing daemon must not break dhclient-script.

use std::collections::HashMap;
use std::time::Duration;

use zbus::Connection;

const SERVICE: &str = "org.freedesktop.NetworkManager";
const PATH: &str = "/org/freedesktop/NetworkManager/Dhcp";
const INTERFACE: &str = "org.freedesktop.NetworkManager.Dhcp";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // dhclient-script must never block on a wedged daemon or bus.
    let _ = tokio::time::timeout(Duration::from_secs(5), notify()).await;
}

async fn notify() {
    let reason = std::env::var("reason").unwrap_or_default();
    let ifname = std::env::var("interface").unwrap_or_default();
    if reason.is_empty() || ifname.is_empty() {
        return;
    }
    let mut data = HashMap::new();
    for (source, target) in [
        ("new_ip_address", "ip_address"),
        ("new_subnet_mask", "subnet_mask"),
        ("new_routers", "routers"),
        ("new_domain_name_servers", "dns"),
        ("new_domain_name", "domain_name"),
        ("new_dhcp_lease_time", "lease_time"),
    ] {
        if let Ok(value) = std::env::var(source)
            && !value.is_empty()
        {
            data.insert(target.to_string(), value);
        }
    }

    let connection = match std::env::var("NM_FREEBSD_BUS_ADDRESS") {
        Ok(address) if !address.is_empty() => {
            zbus::connection::Builder::address(address.as_str())
                .build()
                .await
        }
        _ => Connection::system().await,
    };
    let connection = match connection {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("nm-notify-dhcp: cannot connect to the bus: {error}");
            return;
        }
    };
    let proxy = match zbus::Proxy::new(&connection, SERVICE, PATH, INTERFACE).await {
        Ok(proxy) => proxy,
        Err(error) => {
            eprintln!("nm-notify-dhcp: cannot create proxy: {error}");
            return;
        }
    };
    if let Err(error) = proxy
        .call::<_, _, ()>("Notify", &(ifname.as_str(), reason.as_str(), data))
        .await
    {
        eprintln!("nm-notify-dhcp: Notify failed: {error}");
    }
}
