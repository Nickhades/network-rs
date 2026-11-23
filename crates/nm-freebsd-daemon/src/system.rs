//! Small FreeBSD system helpers built on libc, getifaddrs and /etc files.
//!
//! Anything that touches the kernel directly lives here so the D-Bus layer
//! never has to know about sockaddrs.

use std::ffi::CStr;
use std::net::Ipv4Addr;
use std::ptr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ipv4Address {
    pub address: Ipv4Addr,
    pub prefix: u8,
}

/// Unique interface names, in stable order. Uses getifaddrs so it works even
/// when netlink is unavailable.
pub fn interface_names() -> Vec<String> {
    let mut names = Vec::new();
    let mut addrs: *mut libc::ifaddrs = ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut addrs) } != 0 {
        return names;
    }
    let mut current = addrs;
    while !current.is_null() {
        let entry = unsafe { &*current };
        if !entry.ifa_name.is_null() {
            let name = unsafe { CStr::from_ptr(entry.ifa_name) }
                .to_string_lossy()
                .into_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        current = unsafe { (*current).ifa_next };
    }
    unsafe { libc::freeifaddrs(addrs) };
    names.sort();
    names
}

pub fn ifindex(name: &str) -> Option<i32> {
    let mut buf = [0i8; libc::IFNAMSIZ];
    if name.len() >= libc::IFNAMSIZ {
        return None;
    }
    for (dst, src) in buf.iter_mut().zip(name.as_bytes()) {
        *dst = *src as i8;
    }
    let index = unsafe { libc::if_nametoindex(buf.as_ptr()) };
    if index == 0 { None } else { Some(index as i32) }
}

/// IPv4 addresses currently assigned to an interface.
pub fn ipv4_addresses(ifname: &str) -> Vec<Ipv4Address> {
    let mut out = Vec::new();
    let mut addrs: *mut libc::ifaddrs = ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut addrs) } != 0 {
        return out;
    }
    let mut current = addrs;
    while !current.is_null() {
        let entry = unsafe { &*current };
        if entry.ifa_name.is_null() || entry.ifa_addr.is_null() {
            current = entry.ifa_next;
            continue;
        }
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }.to_string_lossy();
        if name != ifname {
            current = entry.ifa_next;
            continue;
        }
        let family = unsafe { (*entry.ifa_addr).sa_family as i32 };
        if family == libc::AF_INET {
            let sin = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
            let bytes = sin.sin_addr.s_addr.to_ne_bytes();
            let address = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
            let mut prefix = 0u8;
            if !entry.ifa_netmask.is_null() {
                let mask = unsafe { &*(entry.ifa_netmask as *const libc::sockaddr_in) };
                let mask_bytes = mask.sin_addr.s_addr.to_ne_bytes();
                prefix = u32::from_be_bytes(mask_bytes).leading_ones() as u8;
            }
            out.push(Ipv4Address { address, prefix });
        }
        current = entry.ifa_next;
    }
    unsafe { libc::freeifaddrs(addrs) };
    out
}

/// Read nameservers from resolv.conf. Duplicates are removed and order kept.
pub fn dns_servers() -> Vec<String> {
    let mut servers = Vec::new();
    if let Ok(contents) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in contents.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("nameserver") {
                let server = rest.trim();
                if !server.is_empty() && !servers.iter().any(|s| s == server) {
                    servers.push(server.to_string());
                }
            }
        }
    }
    servers
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return "localhost".to_string();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Wi-Fi interfaces to manage. Explicit configuration always wins; otherwise
/// FreeBSD's conventionally named wlan* clones are used.
pub fn discover_wifi_interfaces(configured: &[String]) -> Vec<String> {
    if !configured.is_empty() {
        return configured.to_vec();
    }
    let mut wifi: Vec<String> = interface_names()
        .into_iter()
        .filter(|name| name.starts_with("wlan"))
        .collect();
    wifi.sort();
    wifi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_index_agree() {
        let names = interface_names();
        assert!(!names.is_empty());
        for name in &names {
            assert!(ifindex(name).is_some(), "missing ifindex for {name}");
        }
    }

    #[cfg(target_os = "freebsd")]
    #[test]
    fn loopback_has_index() {
        assert!(interface_names().iter().any(|n| n == "lo0"));
        assert!(ifindex("lo0").is_some());
    }

    #[test]
    fn hostname_is_not_empty() {
        assert!(!hostname().is_empty());
    }
}
