//! Minimal parser/writer for FreeBSD's /etc/wpa_supplicant.conf.
//!
//! The file is the source of truth for Wi-Fi networks. Only network blocks are
//! interpreted; global lines and comments are preserved verbatim.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WpaNetwork {
    pub ssid: Vec<u8>,
    pub psk: Option<String>,
    pub key_mgmt: String,
    pub id_str: Option<String>,
    pub hidden: bool,
    pub disabled: bool,
    /// Unknown keys such as proto/pairwise, preserved for round-trips.
    pub extra: Vec<(String, String)>,
}

impl Default for WpaNetwork {
    fn default() -> Self {
        Self {
            ssid: Vec::new(),
            psk: None,
            key_mgmt: "NONE".to_string(),
            id_str: None,
            hidden: false,
            disabled: false,
            extra: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct WpaConfig {
    /// Non-network lines, preserved in their original order.
    pub preamble: String,
    pub networks: Vec<WpaNetwork>,
}

impl WpaConfig {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let bytes = fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);
        Self::parse(&text)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut preamble = String::new();
        let mut networks = Vec::new();
        let mut in_network = false;
        let mut block = String::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if !in_network {
                if is_network_start(trimmed) {
                    in_network = true;
                    block.clear();
                } else {
                    preamble.push_str(line);
                    preamble.push('\n');
                }
                continue;
            }

            if trimmed == "}" {
                networks.push(parse_block(&block)?);
                in_network = false;
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            block.push_str(trimmed);
            block.push('\n');
        }

        if in_network {
            return Err("unterminated network block".to_string());
        }
        Ok(Self { preamble, networks })
    }

    pub fn render(&self) -> String {
        let mut out = self.preamble.clone();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        for network in &self.networks {
            out.push_str("\nnetwork={\n");
            out.push_str("    ssid=");
            out.push_str(&quote(&String::from_utf8_lossy(&network.ssid)));
            out.push('\n');
            if let Some(psk) = &network.psk {
                out.push_str("    psk=");
                out.push_str(&quote(psk));
                out.push('\n');
            }
            out.push_str("    key_mgmt=");
            out.push_str(&network.key_mgmt);
            out.push('\n');
            if let Some(id_str) = &network.id_str {
                out.push_str("    id_str=");
                out.push_str(&quote(id_str));
                out.push('\n');
            }
            if network.hidden {
                out.push_str("    scan_ssid=1\n");
            }
            if network.disabled {
                out.push_str("    disabled=1\n");
            }
            for (key, value) in &network.extra {
                out.push_str("    ");
                out.push_str(key);
                out.push('=');
                out.push_str(value);
                out.push('\n');
            }
            out.push_str("}\n");
        }
        out
    }

    pub fn upsert(&mut self, network: WpaNetwork) {
        let existing =
            self.networks
                .iter_mut()
                .find(|current| match (&current.id_str, &network.id_str) {
                    (Some(left), Some(right)) => left == right,
                    _ => current.ssid == network.ssid,
                });
        match existing {
            Some(current) => *current = network,
            None => self.networks.push(network),
        }
    }

    pub fn remove(&mut self, ssid: &[u8], id_str: Option<&str>) -> bool {
        let before = self.networks.len();
        self.networks.retain(|network| {
            if let Some(id_str) = id_str
                && network.id_str.as_deref() == Some(id_str)
            {
                return false;
            }
            network.ssid != ssid
        });
        before != self.networks.len()
    }

    /// Write a 0600 config atomically and keep one .bak copy.
    pub fn write_atomic(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() {
            let backup = backup_path(path);
            fs::copy(path, backup)?;
        }
        let tmp = temp_path(path);
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(self.render().as_bytes())?;
        file.sync_all()?;
        fs::rename(tmp, path)?;
        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".nm-freebsd.bak");
    PathBuf::from(name)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".tmp.{}", std::process::id()));
    PathBuf::from(name)
}

fn is_network_start(trimmed: &str) -> bool {
    let no_space: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    no_space.starts_with("network={")
}

fn parse_block(block: &str) -> Result<WpaNetwork, String> {
    let mut network = WpaNetwork::default();
    let mut has_ssid = false;
    for line in block.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_end_matches(';');
        match key {
            "ssid" => {
                network.ssid = unquote(value).into_bytes();
                has_ssid = true;
            }
            "psk" => network.psk = Some(unquote(value)),
            "key_mgmt" => network.key_mgmt = unquote(value),
            "id_str" => network.id_str = Some(unquote(value)),
            "scan_ssid" => network.hidden = value.trim() == "1",
            "disabled" => network.disabled = value.trim() == "1",
            _ => network.extra.push((key.to_string(), value.to_string())),
        }
    }
    if !has_ssid || network.ssid.is_empty() {
        return Err("network block without ssid".to_string());
    }
    Ok(network)
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let value = if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        &value[1..value.len().saturating_sub(1)]
    } else {
        value
    };
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const SAMPLE: &str = "ctrl_interface=/var/run/wpa_supplicant\nap_scan=1\n\nnetwork={\n    ssid=\"云涧\"\n    psk=\"88888888\"\n    proto=RSN\n    key_mgmt=WPA-PSK\n    pairwise=CCMP\n}\n";

    #[test]
    fn parses_real_wpa_block() {
        let config = WpaConfig::parse(SAMPLE).unwrap();
        assert_eq!(config.networks.len(), 1);
        assert_eq!(config.networks[0].ssid, "云涧".as_bytes());
        assert_eq!(config.networks[0].psk.as_deref(), Some("88888888"));
        assert_eq!(config.networks[0].key_mgmt, "WPA-PSK");
        assert!(config.preamble.contains("ap_scan=1"));
        assert!(
            config.networks[0]
                .extra
                .iter()
                .any(|(key, value)| key == "pairwise" && value == "CCMP")
        );
    }

    #[test]
    fn roundtrips_quotes_and_id_str() {
        let mut config = WpaConfig::parse(SAMPLE).unwrap();
        let mut network = config.networks[0].clone();
        network.id_str = Some("uuid-1".to_string());
        network.hidden = true;
        config.upsert(network);
        let text = config.render();
        assert!(text.contains("ssid=\"云涧\""));
        assert!(text.contains("id_str=\"uuid-1\""));
        assert!(text.contains("scan_ssid=1"));
        let reparsed = WpaConfig::parse(&text).unwrap();
        assert_eq!(reparsed.networks[0].id_str.as_deref(), Some("uuid-1"));
        assert!(reparsed.networks[0].hidden);
    }

    #[test]
    fn removes_by_id_or_ssid() {
        let mut config = WpaConfig::parse(SAMPLE).unwrap();
        assert!(!config.remove("不存在的".as_bytes(), None));
        assert!(config.remove("云涧".as_bytes(), None));
        assert!(config.networks.is_empty());
    }

    #[test]
    fn atomic_write_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wpa_supplicant.conf");
        let config = WpaConfig::parse(SAMPLE).unwrap();
        config.write_atomic(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            !path
                .with_file_name("wpa_supplicant.conf.nm-freebsd.bak")
                .exists()
        );
        config.write_atomic(&path).unwrap();
        assert!(
            dir.path()
                .join("wpa_supplicant.conf.nm-freebsd.bak")
                .exists()
        );
    }
}
