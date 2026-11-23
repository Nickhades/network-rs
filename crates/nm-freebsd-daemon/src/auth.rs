//! Caller authorization for the system-bus service.
//!
//! The D-Bus policy is the first gate. This module is the second: it resolves
//! the caller's Unix uid, checks membership in the administrator groups, and
//! gives the Manager.GetPermissions method a truthful answer.

use std::collections::HashMap;
use std::ffi::CString;

use zbus::message::Header;
use zbus::names::BusName;
use zbus::{Connection, fdo};

/// Groups allowed to change network state. The D-Bus policy grants the same
/// groups full method access; keeping both in sync is intentional.
pub const ADMIN_GROUPS: [&str; 2] = ["wheel", "network"];

/// Permission names used by NetworkManager frontends.
pub const PERMISSION_NAMES: [&str; 11] = [
    "org.freedesktop.NetworkManager.enable-disable-network",
    "org.freedesktop.NetworkManager.enable-disable-wifi",
    "org.freedesktop.NetworkManager.enable-disable-wwan",
    "org.freedesktop.NetworkManager.sleep-wake",
    "org.freedesktop.NetworkManager.wifi-share",
    "org.freedesktop.NetworkManager.wifi-share-open",
    "org.freedesktop.NetworkManager.wifi-scan",
    "org.freedesktop.NetworkManager.network-control",
    "org.freedesktop.NetworkManager.settings-modify-system",
    "org.freedesktop.NetworkManager.settings-modify-own",
    "org.freedesktop.NetworkManager.wifi-connect",
];

pub async fn caller_uid(connection: &Connection, header: Header<'_>) -> fdo::Result<u32> {
    let sender = header
        .sender()
        .ok_or_else(|| fdo::Error::Failed("caller has no unique bus name".to_string()))?;
    let bus_name = BusName::try_from(sender.as_str().to_owned())
        .map_err(|error| fdo::Error::Failed(format!("invalid caller bus name: {error}")))?;
    let dbus = fdo::DBusProxy::new(connection).await?;
    dbus.get_connection_unix_user(bus_name).await
}

pub async fn require_admin_uid(
    connection: &Connection,
    header: Header<'_>,
    operation: &str,
) -> fdo::Result<u32> {
    let uid = caller_uid(connection, header).await?;
    if is_admin_uid(uid) {
        Ok(uid)
    } else {
        tracing::warn!(
            uid,
            operation,
            "denied unprivileged NetworkManager operation"
        );
        Err(fdo::Error::AccessDenied(format!(
            "{operation} requires membership in one of: {}",
            ADMIN_GROUPS.join(", ")
        )))
    }
}

pub async fn require_activation_access(
    connection: &Connection,
    header: Header<'_>,
    owner_uid: Option<u32>,
    operation: &str,
) -> fdo::Result<()> {
    let uid = caller_uid(connection, header).await?;
    if activation_access_allowed(uid, owner_uid, is_admin_uid(uid)) {
        Ok(())
    } else {
        tracing::warn!(
            uid,
            ?owner_uid,
            operation,
            "denied activation of a profile owned by another user"
        );
        Err(fdo::Error::AccessDenied(format!(
            "{operation} requires the profile owner or an administrator"
        )))
    }
}

/// Pure owner/admin decision for activation and disconnection.
pub fn activation_access_allowed(caller_uid: u32, owner_uid: Option<u32>, admin: bool) -> bool {
    admin || owner_uid == Some(caller_uid)
}

pub async fn require_settings_write_access(
    connection: &Connection,
    header: Header<'_>,
    owner_uid: Option<u32>,
    operation: &str,
) -> fdo::Result<()> {
    let uid = caller_uid(connection, header).await?;
    if settings_write_access_allowed(uid, owner_uid, is_admin_uid(uid)) {
        Ok(())
    } else {
        tracing::warn!(
            uid,
            ?owner_uid,
            operation,
            "denied settings write to a profile owned by another user"
        );
        Err(fdo::Error::AccessDenied(format!(
            "{operation} requires the profile owner or an administrator"
        )))
    }
}

pub fn settings_write_access_allowed(caller_uid: u32, owner_uid: Option<u32>, admin: bool) -> bool {
    admin || owner_uid == Some(caller_uid)
}

pub async fn require_secret_access(
    connection: &Connection,
    header: Header<'_>,
    owner_uid: Option<u32>,
    operation: &str,
) -> fdo::Result<()> {
    let uid = caller_uid(connection, header).await?;
    if secret_access_allowed(uid, owner_uid, is_admin_uid(uid)) {
        Ok(())
    } else {
        tracing::warn!(
            uid,
            ?owner_uid,
            operation,
            "denied secret access to a profile owned by another user"
        );
        Err(fdo::Error::AccessDenied(format!(
            "{operation} requires the profile owner or an administrator"
        )))
    }
}

/// Pure owner/admin decision, kept separate from NSS for tests.
pub fn secret_access_allowed(caller_uid: u32, owner_uid: Option<u32>, admin: bool) -> bool {
    admin || owner_uid == Some(caller_uid)
}

pub async fn require_admin(
    connection: &Connection,
    header: Header<'_>,
    operation: &str,
) -> fdo::Result<()> {
    let uid = caller_uid(connection, header).await?;
    if is_admin_uid(uid) {
        Ok(())
    } else {
        tracing::warn!(
            uid,
            operation,
            "denied unprivileged NetworkManager operation"
        );
        Err(fdo::Error::AccessDenied(format!(
            "{operation} requires membership in one of: {}",
            ADMIN_GROUPS.join(", ")
        )))
    }
}

pub async fn permissions(
    connection: &Connection,
    header: Header<'_>,
) -> fdo::Result<HashMap<String, String>> {
    let admin = match caller_uid(connection, header).await {
        Ok(uid) => is_admin_uid(uid),
        Err(error) => {
            // GetPermissions is deliberately informational. If the bus cannot
            // resolve the uid, fail closed rather than claiming admin.
            tracing::debug!(%error, "could not resolve GetPermissions caller uid");
            false
        }
    };
    Ok(permission_map(admin))
}

/// Operations that the daemon does not implement at all. Reporting "yes"
/// here would make a frontend show controls that cannot work.
const UNSUPPORTED: [&str; 3] = [
    "org.freedesktop.NetworkManager.enable-disable-wwan",
    "org.freedesktop.NetworkManager.wifi-share",
    "org.freedesktop.NetworkManager.wifi-share-open",
];

pub fn permission_map(admin: bool) -> HashMap<String, String> {
    PERMISSION_NAMES
        .iter()
        .map(|name| {
            let supported = !UNSUPPORTED.contains(name);
            let allowed = supported
                && (admin
                    || *name == "org.freedesktop.NetworkManager.wifi-scan"
                    || *name == "org.freedesktop.NetworkManager.settings-modify-own");
            (
                (*name).to_string(),
                if allowed { "yes" } else { "no" }.to_string(),
            )
        })
        .collect()
}

/// Whether a uid is root or belongs to one of ADMIN_GROUPS.
pub fn is_admin_uid(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let Some((primary_gid, groups)) = user_groups(uid) else {
        return false;
    };
    let wheel = group_gid("wheel");
    let network = group_gid("network");
    admin_from_groups(uid, primary_gid, &groups, wheel, network)
}

/// Pure membership decision, kept separate from NSS so tests do not depend on
/// the host's group database.
pub fn admin_from_groups<G>(
    uid: u32,
    primary_gid: G,
    groups: &[G],
    wheel_gid: Option<G>,
    network_gid: Option<G>,
) -> bool
where
    G: Copy + PartialEq,
{
    if uid == 0 {
        return true;
    }
    let is_member =
        |gid: Option<G>| gid.is_some_and(|gid| primary_gid == gid || groups.contains(&gid));
    is_member(wheel_gid) || is_member(network_gid)
}

fn group_gid(name: &str) -> Option<libc::gid_t> {
    let name = CString::new(name).ok()?;
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let mut group: libc::group = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::group = std::ptr::null_mut();
    let rc = unsafe {
        libc::getgrnam_r(
            name.as_ptr(),
            &mut group,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if rc == 0 && !result.is_null() {
        Some(group.gr_gid)
    } else {
        None
    }
}

fn user_groups(uid: u32) -> Option<(libc::gid_t, Vec<libc::gid_t>)> {
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(
            uid as libc::uid_t,
            &mut passwd,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    let primary_gid = passwd.pw_gid;
    let mut groups = vec![0 as libc::gid_t; 32];
    let mut ngroups = groups.len() as libc::c_int;
    let rc = unsafe {
        libc::getgrouplist(
            passwd.pw_name,
            passwd.pw_gid,
            groups.as_mut_ptr(),
            &mut ngroups,
        )
    };
    if rc == -1 && ngroups > 0 {
        groups.resize(ngroups as usize, 0);
        let rc = unsafe {
            libc::getgrouplist(
                passwd.pw_name,
                passwd.pw_gid,
                groups.as_mut_ptr(),
                &mut ngroups,
            )
        };
        if rc == -1 {
            return None;
        }
    }
    groups.truncate(ngroups.max(0) as usize);
    Some((primary_gid, groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_admin() {
        assert!(admin_from_groups(0, 0, &[], None, None));
    }

    #[test]
    fn wheel_and_network_are_admin() {
        assert!(admin_from_groups(1001, 1001, &[0], Some(0), Some(69)));
        assert!(admin_from_groups(1001, 69, &[], Some(0), Some(69)));
        assert!(admin_from_groups(1001, 1001, &[0, 69], Some(0), Some(69)));
    }

    #[test]
    fn unrelated_group_is_not_admin() {
        assert!(!admin_from_groups(1001, 1001, &[1002], Some(0), Some(69)));
    }

    #[test]
    fn activation_access_is_owner_or_admin() {
        assert!(activation_access_allowed(1001, Some(1001), false));
        assert!(!activation_access_allowed(1002, Some(1001), false));
        assert!(!activation_access_allowed(1001, None, false));
        assert!(activation_access_allowed(1002, Some(1001), true));
        assert!(activation_access_allowed(0, None, true));
    }

    #[test]
    fn settings_write_is_owner_or_admin() {
        assert!(settings_write_access_allowed(1001, Some(1001), false));
        assert!(!settings_write_access_allowed(1002, Some(1001), false));
        assert!(!settings_write_access_allowed(1001, None, false));
        assert!(settings_write_access_allowed(1002, Some(1001), true));
    }

    #[test]
    fn secret_access_is_owner_or_admin() {
        assert!(secret_access_allowed(1001, Some(1001), false));
        assert!(!secret_access_allowed(1002, Some(1001), false));
        assert!(!secret_access_allowed(1001, None, false));
        assert!(secret_access_allowed(1002, Some(1001), true));
        assert!(secret_access_allowed(0, None, true));
    }

    #[test]
    fn permissions_are_truthful() {
        let admin = permission_map(true);
        assert_eq!(admin.len(), PERMISSION_NAMES.len());
        assert_eq!(
            admin["org.freedesktop.NetworkManager.network-control"],
            "yes"
        );
        assert_eq!(
            admin["org.freedesktop.NetworkManager.enable-disable-network"],
            "yes"
        );
        assert_eq!(admin["org.freedesktop.NetworkManager.wifi-share"], "no");
        assert_eq!(
            admin["org.freedesktop.NetworkManager.settings-modify-own"],
            "yes"
        );
        assert_eq!(
            admin["org.freedesktop.NetworkManager.settings-modify-system"],
            "yes"
        );

        let user = permission_map(false);
        assert_eq!(user.len(), PERMISSION_NAMES.len());
        assert_eq!(user["org.freedesktop.NetworkManager.wifi-scan"], "yes");
        assert_eq!(user["org.freedesktop.NetworkManager.network-control"], "no");
        assert_eq!(user["org.freedesktop.NetworkManager.wifi-connect"], "no");
        assert_eq!(
            user["org.freedesktop.NetworkManager.settings-modify-own"],
            "yes"
        );
        assert_eq!(
            user["org.freedesktop.NetworkManager.settings-modify-system"],
            "no"
        );
    }
}
