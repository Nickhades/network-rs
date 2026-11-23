//! Parse the security flags that wpa_supplicant prints in SCAN_RESULTS.

use crate::consts::ap_security_flags as sec;

/// Security-relevant subset of an access point beacon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApSecurity {
    pub privacy: bool,
    pub wpa_flags: u32,
    pub rsn_flags: u32,
}

impl ApSecurity {
    pub fn has_wpa(&self) -> bool {
        self.wpa_flags != 0
    }

    pub fn has_rsn(&self) -> bool {
        self.rsn_flags != 0
    }

    pub fn is_open(&self) -> bool {
        !self.privacy && self.wpa_flags == 0 && self.rsn_flags == 0
    }

    pub fn has_sae(&self) -> bool {
        self.rsn_flags & sec::KEY_MGMT_SAE != 0 || self.wpa_flags & sec::KEY_MGMT_SAE != 0
    }

    pub fn has_psk(&self) -> bool {
        self.rsn_flags & sec::KEY_MGMT_PSK != 0 || self.wpa_flags & sec::KEY_MGMT_PSK != 0
    }

    pub fn has_enterprise(&self) -> bool {
        self.rsn_flags & sec::KEY_MGMT_802_1X != 0 || self.wpa_flags & sec::KEY_MGMT_802_1X != 0
    }

    pub fn has_owe(&self) -> bool {
        self.rsn_flags & sec::KEY_MGMT_OWE != 0 || self.wpa_flags & sec::KEY_MGMT_OWE != 0
    }

    /// The key management suite a new profile should use.
    pub fn preferred_key_mgmt(&self) -> KeyManagement {
        if self.is_open() {
            KeyManagement::None
        } else if self.has_sae() && !self.has_psk() {
            KeyManagement::Sae
        } else if self.has_psk() {
            KeyManagement::WpaPsk
        } else if self.has_owe() {
            KeyManagement::Owe
        } else if self.has_enterprise() {
            KeyManagement::WpaEap
        } else if self.privacy {
            KeyManagement::Wep
        } else {
            KeyManagement::None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyManagement {
    None,
    WpaPsk,
    Sae,
    WpaEap,
    Owe,
    Wep,
}

impl KeyManagement {
    pub fn wpa_config_value(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::WpaPsk => "WPA-PSK",
            Self::Sae => "SAE",
            Self::WpaEap => "WPA-EAP",
            Self::Owe => "OWE",
            Self::Wep => "NONE",
        }
    }

    pub fn needs_passphrase(self) -> bool {
        matches!(self, Self::WpaPsk | Self::Sae)
    }

    pub fn is_enterprise(self) -> bool {
        matches!(self, Self::WpaEap)
    }
}

/// Parse wpa_supplicant's flag field, for example:
///
/// * "[WPA2-PSK-CCMP][ESS]"
/// * "[WPA-PSK-TKIP][WPA2-PSK-CCMP][ESS]"
/// * "[RSN-SAE-CCMP][ESS]"
///
/// Unknown cipher names are ignored rather than treated as an error; a new
/// cipher must not hide the access point from the list.
pub fn parse_scan_flags(raw: &str) -> ApSecurity {
    let upper = raw.to_ascii_uppercase();
    let mut out = ApSecurity::default();

    for group in bracket_groups(&upper) {
        let mut parts = group.split(['-', '+']);
        let proto = parts.next().unwrap_or("").trim();
        let mut target = match proto {
            p if p.starts_with("WPA2") || p.starts_with("RSN") => Some(&mut out.rsn_flags),
            p if p.starts_with("WPA") => Some(&mut out.wpa_flags),
            "SAE" => {
                out.privacy = true;
                out.rsn_flags |= sec::KEY_MGMT_SAE;
                None
            }
            "OWE" => {
                out.privacy = true;
                out.rsn_flags |= sec::KEY_MGMT_OWE;
                None
            }
            p if p.starts_with("WEP") => {
                out.privacy = true;
                out.wpa_flags |= sec::PAIR_WEP40;
                None
            }
            _ => None,
        };

        if proto.starts_with("WPA") || proto.starts_with("RSN") {
            out.privacy = true;
        }

        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(flags) = target.as_deref_mut() {
                match part {
                    "PSK" => *flags |= sec::KEY_MGMT_PSK,
                    "EAP" | "802.1X" => *flags |= sec::KEY_MGMT_802_1X,
                    "SAE" => *flags |= sec::KEY_MGMT_SAE,
                    "OWE" => *flags |= sec::KEY_MGMT_OWE,
                    "CCMP" => *flags |= sec::PAIR_CCMP,
                    "TKIP" => *flags |= sec::PAIR_TKIP,
                    "WEP40" => *flags |= sec::PAIR_WEP40,
                    "WEP104" => *flags |= sec::PAIR_WEP104,
                    _ => {}
                }
            }
        }
    }

    out
}

fn bracket_groups(input: &str) -> Vec<&str> {
    let mut groups = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find('[') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find(']') {
            groups.push(&after[..end]);
            rest = &after[end + 1..];
        } else {
            groups.push(after);
            break;
        }
    }
    if groups.is_empty() {
        groups.push(input);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_network() {
        let s = parse_scan_flags("[ESS]");
        assert!(!s.privacy);
        assert_eq!(s.preferred_key_mgmt(), KeyManagement::None);
        assert!(s.is_open());
    }

    #[test]
    fn wpa2_psk() {
        let s = parse_scan_flags("[WPA2-PSK-CCMP][ESS]");
        assert!(s.privacy);
        assert_eq!(s.rsn_flags & sec::KEY_MGMT_PSK, sec::KEY_MGMT_PSK);
        assert_eq!(s.rsn_flags & sec::PAIR_CCMP, sec::PAIR_CCMP);
        assert_eq!(s.preferred_key_mgmt(), KeyManagement::WpaPsk);
        assert!(s.has_psk());
    }

    #[test]
    fn wpa_and_wpa2() {
        let s = parse_scan_flags("[WPA-PSK-TKIP][WPA2-PSK-CCMP][ESS]");
        assert_eq!(s.wpa_flags & sec::PAIR_TKIP, sec::PAIR_TKIP);
        assert_eq!(s.wpa_flags & sec::KEY_MGMT_PSK, sec::KEY_MGMT_PSK);
        assert_eq!(s.rsn_flags & sec::PAIR_CCMP, sec::PAIR_CCMP);
        assert_eq!(s.rsn_flags & sec::KEY_MGMT_PSK, sec::KEY_MGMT_PSK);
    }

    #[test]
    fn wpa3_sae() {
        let s = parse_scan_flags("[RSN-SAE-CCMP][ESS]");
        assert_eq!(s.preferred_key_mgmt(), KeyManagement::Sae);
        assert!(s.has_sae());
        assert!(s.privacy);
    }

    #[test]
    fn enterprise() {
        let s = parse_scan_flags("[WPA2-EAP-CCMP][ESS]");
        assert!(s.has_enterprise());
        assert_eq!(s.preferred_key_mgmt(), KeyManagement::WpaEap);
    }

    #[test]
    fn wep() {
        let s = parse_scan_flags("[WEP][ESS]");
        assert!(s.privacy);
        assert_eq!(s.preferred_key_mgmt(), KeyManagement::Wep);
    }

    #[test]
    fn unknown_cipher_keeps_network() {
        let s = parse_scan_flags("[WPA2-PSK-CCMP+GCMP256][ESS]");
        assert!(s.has_psk());
        assert_eq!(s.rsn_flags & sec::PAIR_CCMP, sec::PAIR_CCMP);
    }

    #[test]
    fn flags_without_brackets() {
        let s = parse_scan_flags("WPA2-PSK-CCMP");
        assert!(s.has_psk());
    }
}
