//! DHCP client lifecycle.
//!
//! The default backend is FreeBSD's base `/sbin/dhclient`; `dhcpcd` is kept
//! optional. The daemon never rewrites `/etc` or inspects the lease file: an
//! installed dhclient exit hook pushes BOUND/RENEW/EXPIRE events over D-Bus.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::{Config, DhcpBackendKind};

#[derive(Debug, Error)]
pub enum DhcpError {
    #[error("DHCP backend is disabled")]
    Disabled,
    #[error("{backend} is not installed or not executable")]
    Missing { backend: &'static str },
    #[error("failed to start {backend}: {source}")]
    Spawn {
        backend: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("the {backend} process did not expose a pid")]
    NoPid { backend: &'static str },
    #[error("already starting {0}")]
    Busy(String),
}

#[derive(Debug, Clone)]
pub enum DhcpEvent {
    Started {
        ifname: String,
        managed: bool,
        backend: &'static str,
    },
    Exited {
        ifname: String,
        pid: u32,
        code: Option<i32>,
    },
    Error {
        ifname: String,
        message: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct DhcpStart {
    pub managed: bool,
    pub backend: &'static str,
}

pub struct DhcpManager {
    kind: DhcpBackendKind,
    dhclient_path: PathBuf,
    dhcpcd_path: PathBuf,
    tx: mpsc::Sender<DhcpEvent>,
    children: Mutex<HashMap<String, u32>>,
    reused: Mutex<HashSet<String>>,
}

impl DhcpManager {
    pub fn new(config: &Config, tx: mpsc::Sender<DhcpEvent>) -> Self {
        let kind = resolve_backend(
            &config.dhcp_backend,
            &config.dhclient_path,
            &config.dhcpcd_path,
        );
        tracing::info!(
            backend = backend_name(kind),
            dhclient = %config.dhclient_path.display(),
            dhcpcd = %config.dhcpcd_path.display(),
            "DHCP backend selected"
        );
        Self {
            kind,
            dhclient_path: config.dhclient_path.clone(),
            dhcpcd_path: config.dhcpcd_path.clone(),
            tx,
            children: Mutex::new(HashMap::new()),
            reused: Mutex::new(HashSet::new()),
        }
    }

    pub fn backend(&self) -> &'static str {
        backend_name(self.kind)
    }

    pub fn is_disabled(&self) -> bool {
        self.kind == DhcpBackendKind::Disabled
    }

    /// Start a DHCP client for `ifname`, or attach to an existing dhclient.
    ///
    /// A PID file for an already-running system dhclient is reused only when
    /// `replace_existing` is false (wired setups).  A Wi-Fi activation has
    /// just switched networks behind that client, so it must be stopped and
    /// replaced by a daemon-managed client; otherwise it keeps the old lease
    /// and DISCOVER is never sent for the new AP.
    pub fn start(
        self: &Arc<Self>,
        ifname: &str,
        replace_existing: bool,
    ) -> Result<DhcpStart, DhcpError> {
        if self.kind == DhcpBackendKind::Disabled {
            return Err(DhcpError::Disabled);
        }
        let managed_pid = {
            let children = self.children.lock().expect("dhcp children");
            children.get(ifname).copied()
        };
        if let Some(pid) = managed_pid {
            if !replace_existing {
                return Ok(DhcpStart {
                    managed: true,
                    backend: self.backend(),
                });
            }
            // The managed client was started for the previous SSID.  A new
            // Wi-Fi activation must release/renew against the new AP, so stop
            // it and wait for the old process to exit before starting another.
            tracing::info!(
                ifname,
                pid,
                "restarting managed DHCP client for new Wi-Fi network"
            );
            self.stop(ifname);
            for _ in 0..40 {
                if !pid_alive(pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if let Some(pid) = self.existing_pid(ifname) {
            if replace_existing {
                tracing::info!(
                    ifname,
                    pid,
                    "replacing existing dhclient for new Wi-Fi network"
                );
                // This is normally the rc.conf dhclient that is still bound to
                // the previous SSID.  Wi-Fi activation owns the interface now,
                // so terminate it and start a child that follows this lease.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
                for _ in 0..40 {
                    if !pid_alive(pid) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                self.cleanup_pid_file(ifname, pid);
            } else {
                self.reused
                    .lock()
                    .expect("dhcp reused")
                    .insert(ifname.to_string());
                tracing::info!(ifname, pid, "reusing existing dhclient");
                let _ = self.tx.try_send(DhcpEvent::Started {
                    ifname: ifname.to_string(),
                    managed: false,
                    backend: self.backend(),
                });
                return Ok(DhcpStart {
                    managed: false,
                    backend: self.backend(),
                });
            }
        }

        let (program, args) = match self.kind {
            DhcpBackendKind::Dhclient => (
                self.dhclient_path.clone(),
                vec!["-d".to_string(), ifname.to_string()],
            ),
            DhcpBackendKind::Dhcpcd => (
                self.dhcpcd_path.clone(),
                vec!["-4".to_string(), "-B".to_string(), ifname.to_string()],
            ),
            DhcpBackendKind::Auto | DhcpBackendKind::Disabled => {
                return Err(DhcpError::Disabled);
            }
        };
        if !is_executable(&program) {
            return Err(DhcpError::Missing {
                backend: self.backend(),
            });
        }
        let mut child = Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| DhcpError::Spawn {
                backend: self.backend(),
                source,
            })?;
        let pid = child.id().ok_or(DhcpError::NoPid {
            backend: self.backend(),
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        self.children
            .lock()
            .expect("dhcp children")
            .insert(ifname.to_string(), pid);
        tracing::info!(ifname, pid, backend = self.backend(), "started DHCP client");
        let _ = self.tx.try_send(DhcpEvent::Started {
            ifname: ifname.to_string(),
            managed: true,
            backend: self.backend(),
        });

        let manager = Arc::clone(self);
        let event_ifname = ifname.to_string();
        let backend = self.backend();
        tokio::spawn(async move {
            if let Some(stdout) = stdout {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        tracing::debug!(backend, %line, "DHCP stdout");
                    }
                });
            }
            if let Some(stderr) = stderr {
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        tracing::debug!(backend, %line, "DHCP stderr");
                    }
                });
            }
            let status = child.wait().await;
            let code = status.ok().and_then(|status| status.code());
            {
                let mut children = manager.children.lock().expect("dhcp children");
                if children.get(&event_ifname) == Some(&pid) {
                    children.remove(&event_ifname);
                }
            }
            manager.cleanup_pid_file(&event_ifname, pid);
            tracing::info!(ifname = %event_ifname, pid, code, "DHCP client exited");
            let _ = manager
                .tx
                .send(DhcpEvent::Exited {
                    ifname: event_ifname,
                    pid,
                    code,
                })
                .await;
        });

        Ok(DhcpStart {
            managed: true,
            backend: self.backend(),
        })
    }

    /// Stop only the process this daemon started. A reused system dhclient is
    /// left alone.
    pub fn stop(&self, ifname: &str) {
        let managed = self.children.lock().expect("dhcp children").remove(ifname);
        if let Some(pid) = managed {
            tracing::info!(ifname, pid, "stopping managed DHCP client");
            // SIGTERM lets dhclient run its release path; the monitor task
            // reaps it and emits Exited.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            return;
        }
        if self.reused.lock().expect("dhcp reused").remove(ifname) {
            tracing::info!(ifname, "leaving reused system dhclient running");
        }
    }

    pub fn is_managed(&self, ifname: &str) -> bool {
        self.children
            .lock()
            .expect("dhcp children")
            .contains_key(ifname)
    }

    fn existing_pid(&self, ifname: &str) -> Option<u32> {
        let path = match self.kind {
            DhcpBackendKind::Dhclient => dhclient_pid_path(ifname),
            DhcpBackendKind::Dhcpcd => PathBuf::from(format!("/var/run/dhcpcd/{ifname}.pid")),
            _ => return None,
        };
        let text = std::fs::read_to_string(path).ok()?;
        let pid: u32 = text.trim().parse().ok()?;
        pid_alive(pid).then_some(pid)
    }

    fn cleanup_pid_file(&self, ifname: &str, pid: u32) {
        let path = match self.kind {
            DhcpBackendKind::Dhclient => dhclient_pid_path(ifname),
            DhcpBackendKind::Dhcpcd => PathBuf::from(format!("/var/run/dhcpcd/{ifname}.pid")),
            _ => return,
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        if text.trim() == pid.to_string() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn dhclient_pid_path(ifname: &str) -> PathBuf {
    PathBuf::from(format!("/var/run/dhclient/dhclient.{ifname}.pid"))
}

fn backend_name(kind: DhcpBackendKind) -> &'static str {
    match kind {
        DhcpBackendKind::Auto => "auto",
        DhcpBackendKind::Dhclient => "dhclient",
        DhcpBackendKind::Dhcpcd => "dhcpcd",
        DhcpBackendKind::Disabled => "disabled",
    }
}

fn resolve_backend(requested: &DhcpBackendKind, dhclient: &Path, dhcpcd: &Path) -> DhcpBackendKind {
    match requested {
        DhcpBackendKind::Auto => {
            if is_executable(dhclient) {
                DhcpBackendKind::Dhclient
            } else if find_in_path(dhcpcd).is_some() {
                DhcpBackendKind::Dhcpcd
            } else {
                DhcpBackendKind::Disabled
            }
        }
        DhcpBackendKind::Dhclient if is_executable(dhclient) => DhcpBackendKind::Dhclient,
        DhcpBackendKind::Dhcpcd if find_in_path(dhcpcd).is_some() => DhcpBackendKind::Dhcpcd,
        DhcpBackendKind::Dhclient => DhcpBackendKind::Dhclient,
        DhcpBackendKind::Dhcpcd => DhcpBackendKind::Dhcpcd,
        DhcpBackendKind::Disabled => DhcpBackendKind::Disabled,
    }
}

pub fn find_in_path(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return is_executable(program).then(|| program.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        true
    } else {
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn executable_detection() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("dhclient");
        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        drop(f);
        assert!(!is_executable(&file));
        let mut perms = std::fs::metadata(&file).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&file, perms).unwrap();
        assert!(is_executable(&file));
    }

    #[test]
    fn disabled_backend_refuses_start() {
        let config = Config::for_tests(PathBuf::from("/tmp/wpa"), PathBuf::from("/tmp/state"));
        let (tx, _rx) = mpsc::channel(4);
        let manager = Arc::new(DhcpManager::new(&config, tx));
        assert!(manager.is_disabled());
        assert!(matches!(
            manager.start("wlan0", false),
            Err(DhcpError::Disabled)
        ));
    }

    #[test]
    fn dhclient_pid_path_is_freebsd_layout() {
        assert_eq!(
            dhclient_pid_path("wlan0"),
            PathBuf::from("/var/run/dhclient/dhclient.wlan0.pid")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn managed_client_starts_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("dhclient");
        let marker = dir.path().join("starts");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf 'x\\n' >> '{}'\nsleep 30\n",
                marker.display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let mut config = Config::for_tests(dir.path().join("wpa"), dir.path().join("state"));
        config.dhcp_backend = DhcpBackendKind::Dhclient;
        config.dhclient_path = script;
        let (tx, mut rx) = mpsc::channel(8);
        let manager = Arc::new(DhcpManager::new(&config, tx));
        let started = manager
            .start("nmtest0", false)
            .expect("start fake dhclient");
        assert!(started.managed);
        assert!(manager.is_managed("nmtest0"));
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap(),
            Some(DhcpEvent::Started { managed: true, .. })
        ));
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        assert_eq!(
            std::fs::read_to_string(&marker)
                .unwrap_or_default()
                .lines()
                .count(),
            1,
            "first fake DHCP client should have started"
        );

        // A new Wi-Fi activation must replace the old managed client rather
        // than silently returning the existing one.
        let restarted = manager
            .start("nmtest0", true)
            .expect("restart fake dhclient");
        assert!(restarted.managed);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            std::fs::read_to_string(&marker)
                .unwrap_or_default()
                .lines()
                .count(),
            2
        );

        manager.stop("nmtest0");
        assert!(!manager.is_managed("nmtest0"));
    }
}
