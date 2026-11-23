//! Minimal FreeBSD rtnetlink client.
//!
//! FreeBSD 14+ ships a Linux-compatible netlink(4) implementation. We only
//! need two things from it: a link dump at startup and link/address
//! notifications afterwards. The wire layout is host-endian and stable; the
//! constant values below come from /usr/include/netlink on FreeBSD 15.
//!
//! The socket is deliberately small and synchronous. A dedicated OS thread
//! owns the monitoring socket and forwards events into the async runtime.

use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr;

use tokio::sync::mpsc;

const AF_NETLINK: i32 = 38;
const NETLINK_ROUTE: i32 = 0;

const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_GETLINK: u16 = 18;
const RTM_NEWADDR: u16 = 20;
const RTM_DELADDR: u16 = 21;

const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_DUMP: u16 = 0x0300;

const RTMGRP_LINK: u32 = 0x1;
const RTMGRP_IPV4_IFADDR: u32 = 0x10;

const IFLA_ADDRESS: u16 = 1;
const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_OPERSTATE: u16 = 16;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct NlMsghdr {
    len: u32,
    typ: u16,
    flags: u16,
    seq: u32,
    pid: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct IfInfoMsg {
    family: u8,
    pad: u8,
    typ: u16,
    index: i32,
    flags: u32,
    change: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RtAttr {
    len: u16,
    typ: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SockaddrNl {
    len: u8,
    family: u8,
    pad: u16,
    pid: u32,
    groups: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    pub ifname: String,
    pub ifindex: i32,
    pub mtu: u32,
    pub flags: u32,
    pub operstate: u8,
    pub mac: String,
}

impl LinkInfo {
    pub fn is_up(&self) -> bool {
        self.flags & libc::IFF_UP as u32 != 0
    }

    pub fn is_running(&self) -> bool {
        self.operstate == 6 || self.flags & libc::IFF_RUNNING as u32 != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetlinkEvent {
    /// A link appeared or changed.
    LinkChanged(LinkInfo),
    LinkRemoved {
        ifname: String,
    },
    /// An IPv4/IPv6 address was added or removed on an interface.
    AddressChanged {
        ifname: String,
    },
}

fn socket() -> io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(AF_NETLINK, libc::SOCK_RAW, NETLINK_ROUTE) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn bind(fd: &OwnedFd, groups: u32) -> io::Result<()> {
    let addr = SockaddrNl {
        len: size_of::<SockaddrNl>() as u8,
        family: AF_NETLINK as u8,
        pad: 0,
        pid: 0,
        groups,
    };
    let rc = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            &addr as *const SockaddrNl as *const libc::sockaddr,
            size_of::<SockaddrNl>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_recv_timeout(fd: &OwnedFd, millis: i64) -> io::Result<()> {
    let tv = libc::timeval {
        tv_sec: millis / 1000,
        tv_usec: ((millis % 1000) * 1000) as libc::suseconds_t,
    };
    let rc = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const libc::timeval as *const libc::c_void,
            size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Dump all links via RTM_GETLINK.
pub fn dump_links() -> io::Result<Vec<LinkInfo>> {
    let fd = socket()?;
    bind(&fd, 0)?;
    set_recv_timeout(&fd, 2_000)?;

    let mut request = [0u8; 32];
    let len = 32u32;
    let header = NlMsghdr {
        len,
        typ: RTM_GETLINK,
        flags: NLM_F_REQUEST | NLM_F_DUMP,
        seq: 1,
        pid: 0,
    };
    let ifinfo = IfInfoMsg {
        family: libc::AF_UNSPEC as u8,
        pad: 0,
        typ: 0,
        index: 0,
        flags: 0,
        change: 0,
    };
    unsafe {
        ptr::write_unaligned(request.as_mut_ptr() as *mut NlMsghdr, header);
        ptr::write_unaligned(request.as_mut_ptr().add(16) as *mut IfInfoMsg, ifinfo);
    }

    let dst = SockaddrNl {
        len: size_of::<SockaddrNl>() as u8,
        family: AF_NETLINK as u8,
        pad: 0,
        pid: 0,
        groups: 0,
    };
    let sent = unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            request.as_ptr() as *const libc::c_void,
            request.len(),
            0,
            &dst as *const SockaddrNl as *const libc::sockaddr,
            size_of::<SockaddrNl>() as libc::socklen_t,
        )
    };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut links = Vec::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = unsafe {
            libc::recv(
                fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if n == 0 {
            break;
        }
        let mut offset = 0usize;
        let total = n as usize;
        while offset + size_of::<NlMsghdr>() <= total {
            let header: NlMsghdr =
                unsafe { ptr::read_unaligned(buf.as_ptr().add(offset) as *const NlMsghdr) };
            let msg_len = header.len as usize;
            if msg_len < size_of::<NlMsghdr>() || offset + msg_len > total {
                break;
            }
            let payload = &buf[offset + size_of::<NlMsghdr>()..offset + msg_len];
            match header.typ {
                NLMSG_DONE => return Ok(links),
                NLMSG_ERROR => {
                    if payload.len() >= 4 {
                        let err = i32::from_ne_bytes(payload[..4].try_into().unwrap());
                        if err != 0 {
                            return Err(io::Error::from_raw_os_error(-err));
                        }
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "truncated netlink error",
                    ));
                }
                RTM_NEWLINK => {
                    if let Some(link) = parse_link(payload) {
                        links.push(link);
                    }
                }
                _ => {}
            }
            offset += align4(msg_len);
        }
    }
    Ok(links)
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn parse_link(payload: &[u8]) -> Option<LinkInfo> {
    if payload.len() < size_of::<IfInfoMsg>() {
        return None;
    }
    let ifinfo: IfInfoMsg = unsafe { ptr::read_unaligned(payload.as_ptr() as *const IfInfoMsg) };
    let mut ifname = None;
    let mut mtu = 0u32;
    let mut operstate = 0u8;
    let mut mac = String::new();

    let mut pos = size_of::<IfInfoMsg>();
    while pos + size_of::<RtAttr>() <= payload.len() {
        let attr: RtAttr =
            unsafe { ptr::read_unaligned(payload.as_ptr().add(pos) as *const RtAttr) };
        let attr_len = attr.len as usize;
        if attr_len < size_of::<RtAttr>() || pos + attr_len > payload.len() {
            break;
        }
        let data = &payload[pos + size_of::<RtAttr>()..pos + attr_len];
        match attr.typ {
            IFLA_IFNAME => {
                let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
                ifname = Some(String::from_utf8_lossy(&data[..end]).into_owned());
            }
            IFLA_MTU => {
                if data.len() >= 4 {
                    mtu = u32::from_ne_bytes(data[..4].try_into().unwrap());
                }
            }
            IFLA_OPERSTATE => {
                if let Some(first) = data.first() {
                    operstate = *first;
                }
            }
            IFLA_ADDRESS => {
                if data.len() == 6 {
                    mac = data
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<Vec<_>>()
                        .join(":");
                } else if !data.is_empty() {
                    mac = data.iter().map(|b| format!("{b:02x}")).collect();
                }
            }
            _ => {}
        }
        pos += align4(attr_len);
    }

    Some(LinkInfo {
        ifname: ifname?,
        ifindex: ifinfo.index,
        mtu,
        flags: ifinfo.flags,
        operstate,
        mac,
    })
}

fn parse_link_event(payload: &[u8], removed: bool) -> Option<NetlinkEvent> {
    if removed {
        let link = parse_link(payload)?;
        Some(NetlinkEvent::LinkRemoved {
            ifname: link.ifname,
        })
    } else {
        parse_link(payload).map(NetlinkEvent::LinkChanged)
    }
}

fn parse_addr_event(payload: &[u8]) -> Option<NetlinkEvent> {
    // ifaddrmsg: family, prefixlen, flags, scope, index
    if payload.len() < 8 {
        return None;
    }
    let index = i32::from_ne_bytes(payload[4..8].try_into().ok()?);
    Some(NetlinkEvent::AddressChanged {
        ifname: ifname_for_index(index).unwrap_or_else(|| index.to_string()),
    })
}

/// Resolve an interface index to a name using getifaddrs. Address events are
/// infrequent, so a small scan is cheaper than maintaining another cache.
fn ifname_for_index(index: i32) -> Option<String> {
    crate::system::interface_names()
        .into_iter()
        .find(|name| crate::system::ifindex(name) == Some(index))
}

/// Start the netlink monitor thread. Returns an error if the socket or thread
/// cannot be created so the caller can fall back to polling.
pub fn spawn_monitor(tx: mpsc::Sender<NetlinkEvent>) -> io::Result<()> {
    let fd = socket()?;
    bind(&fd, RTMGRP_LINK | RTMGRP_IPV4_IFADDR)?;
    std::thread::Builder::new()
        .name("nm-netlink".to_string())
        .spawn(move || {
            let mut buf = [0u8; 65_536];
            loop {
                let n = unsafe {
                    libc::recv(
                        fd.as_raw_fd(),
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                        0,
                    )
                };
                if n < 0 {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    tracing::warn!(error = %err, "netlink monitor stopped");
                    return;
                }
                let total = n as usize;
                let mut offset = 0usize;
                while offset + size_of::<NlMsghdr>() <= total {
                    let header: NlMsghdr =
                        unsafe { ptr::read_unaligned(buf.as_ptr().add(offset) as *const NlMsghdr) };
                    let msg_len = header.len as usize;
                    if msg_len < size_of::<NlMsghdr>() || offset + msg_len > total {
                        break;
                    }
                    let payload = &buf[offset + size_of::<NlMsghdr>()..offset + msg_len];
                    let event = match header.typ {
                        RTM_NEWLINK => parse_link_event(payload, false),
                        RTM_DELLINK => parse_link_event(payload, true),
                        RTM_NEWADDR | RTM_DELADDR => parse_addr_event(payload),
                        _ => None,
                    };
                    if let Some(event) = event
                        && tx.blocking_send(event).is_err()
                    {
                        return;
                    }
                    offset += align4(msg_len);
                }
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_link_message(name: &str, index: i32, mtu: u32, mac: &[u8]) -> Vec<u8> {
        let mut attrs = Vec::new();
        // IFLA_IFNAME
        let mut name_attr = Vec::new();
        name_attr.extend_from_slice(name.as_bytes());
        name_attr.push(0);
        push_attr(&mut attrs, IFLA_IFNAME, &name_attr);
        // IFLA_MTU
        push_attr(&mut attrs, IFLA_MTU, &mtu.to_ne_bytes());
        // IFLA_OPERSTATE
        push_attr(&mut attrs, IFLA_OPERSTATE, &[6]);
        // IFLA_ADDRESS
        push_attr(&mut attrs, IFLA_ADDRESS, mac);

        let msg_len = size_of::<NlMsghdr>() + size_of::<IfInfoMsg>() + attrs.len();
        let mut out = Vec::with_capacity(msg_len);
        let header = NlMsghdr {
            len: msg_len as u32,
            typ: RTM_NEWLINK,
            flags: 0,
            seq: 1,
            pid: 0,
        };
        let ifinfo = IfInfoMsg {
            family: libc::AF_UNSPEC as u8,
            pad: 0,
            typ: 6,
            index,
            flags: libc::IFF_UP as u32,
            change: 0,
        };
        out.extend_from_slice(unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(header).cast::<u8>(),
                size_of::<NlMsghdr>(),
            )
        });
        out.extend_from_slice(unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!(ifinfo).cast::<u8>(),
                size_of::<IfInfoMsg>(),
            )
        });
        out.extend_from_slice(&attrs);
        out
    }

    fn push_attr(out: &mut Vec<u8>, typ: u16, data: &[u8]) {
        let len = size_of::<RtAttr>() + data.len();
        out.extend_from_slice(&(len as u16).to_ne_bytes());
        out.extend_from_slice(&typ.to_ne_bytes());
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }

    #[test]
    fn parses_link_message() {
        let raw = build_link_message("wlan0", 3, 1500, &[0xb8, 0x1e, 0xa4, 0xa3, 0x29, 0xe9]);
        let header: NlMsghdr = unsafe { ptr::read_unaligned(raw.as_ptr() as *const NlMsghdr) };
        assert_eq!(header.typ, RTM_NEWLINK);
        let payload = &raw[size_of::<NlMsghdr>()..];
        let link = parse_link(payload).expect("link");
        assert_eq!(link.ifname, "wlan0");
        assert_eq!(link.ifindex, 3);
        assert_eq!(link.mtu, 1500);
        assert_eq!(link.mac, "b8:1e:a4:a3:29:e9");
        assert!(link.is_up());
        assert!(link.is_running());
    }

    #[test]
    fn ignores_truncated_attributes() {
        let mut raw = build_link_message("re0", 1, 1500, &[0, 1, 2, 3, 4, 5]);
        raw.truncate(raw.len() - 2);
        let payload = &raw[size_of::<NlMsghdr>()..];
        let link = parse_link(payload).expect("partial parse");
        assert_eq!(link.ifname, "re0");
    }

    #[cfg(target_os = "freebsd")]
    #[test]
    fn live_link_dump_contains_loopback() {
        let links = dump_links().expect("netlink dump");
        assert!(!links.is_empty());
        assert!(links.iter().any(|l| l.ifname == "lo0"));
        assert!(links.iter().all(|l| l.ifindex > 0));
    }
}
