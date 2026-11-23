//! Desktop environment detection.
//!
//! NetworkManager-compatible frontends are desktop specific: plasma-nm on KDE,
//! gnome-shell's network panel on GNOME, and a panel plugin on XFCE. The
//! daemon itself always speaks D-Bus; this detection only determines whether a
//! compatibility plugin is needed and is logged for troubleshooting.
//!
//! The architecture called for the detect-desktop-environment crate. That
//! crate targets Linux and adds no value over reading the XDG variables
//! directly, so the same rules are implemented here, with unit tests.

use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Kde,
    Gnome,
    Xfce,
    Unknown,
}

impl DesktopEnvironment {
    /// Detect using the current process environment.
    pub fn detect() -> Self {
        Self::detect_from(|key| env::var(key).ok())
    }

    /// Detect using an injected environment accessor. Useful for tests.
    pub fn detect_from<F>(get: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        for key in [
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_DESKTOP",
            "DESKTOP_SESSION",
        ] {
            if let Some(value) = get(key) {
                let parsed = Self::parse(&value);
                if parsed != Self::Unknown {
                    return parsed;
                }
            }
        }
        Self::Unknown
    }

    /// Parse a colon/semicolon/space separated desktop list.
    pub fn parse(value: &str) -> Self {
        for token in value.split([':', ';', ' ', '\t']) {
            match token.trim().to_ascii_lowercase().as_str() {
                "kde" | "plasma" | "plasmashell" | "kde4" | "kde5" | "kde6" => {
                    return Self::Kde;
                }
                "gnome" | "gnome-classic" | "gnome-xorg" | "gnome-wayland" | "unity" => {
                    return Self::Gnome;
                }
                "xfce" | "xfce4" | "xfce4-session" => return Self::Xfce,
                _ => {}
            }
        }
        Self::Unknown
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kde => "kde",
            Self::Gnome => "gnome",
            Self::Xfce => "xfce",
            Self::Unknown => "unknown",
        }
    }

    /// True when a desktop-specific frontend is not expected to connect on its
    /// own, so the XFCE panel plugin is the intended user interface.
    pub fn uses_xfce_frontend(self) -> bool {
        matches!(self, Self::Xfce)
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopEnvironment;
    use std::collections::HashMap;

    fn detect(vars: &[(&str, &str)]) -> DesktopEnvironment {
        let map: HashMap<&str, &str> = vars.iter().copied().collect();
        DesktopEnvironment::detect_from(|key| map.get(key).map(|v| (*v).to_string()))
    }

    #[test]
    fn detects_common_desktops() {
        assert_eq!(
            detect(&[("XDG_CURRENT_DESKTOP", "XFCE")]),
            DesktopEnvironment::Xfce
        );
        assert_eq!(
            detect(&[("XDG_CURRENT_DESKTOP", "KDE")]),
            DesktopEnvironment::Kde
        );
        assert_eq!(
            detect(&[("XDG_CURRENT_DESKTOP", "GNOME")]),
            DesktopEnvironment::Gnome
        );
        assert_eq!(
            detect(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")]),
            DesktopEnvironment::Gnome
        );
        assert_eq!(
            detect(&[("DESKTOP_SESSION", "plasma")]),
            DesktopEnvironment::Kde
        );
        assert_eq!(
            detect(&[("XDG_SESSION_DESKTOP", "xfce")]),
            DesktopEnvironment::Xfce
        );
    }

    #[test]
    fn current_desktop_wins_over_fallback() {
        assert_eq!(
            detect(&[
                ("XDG_CURRENT_DESKTOP", "XFCE"),
                ("DESKTOP_SESSION", "plasma"),
            ]),
            DesktopEnvironment::Xfce
        );
    }

    #[test]
    fn unknown_when_nothing_matches() {
        assert_eq!(detect(&[]), DesktopEnvironment::Unknown);
        assert_eq!(
            detect(&[("XDG_CURRENT_DESKTOP", "sway")]),
            DesktopEnvironment::Unknown
        );
    }

    #[test]
    fn parses_multi_value_lists() {
        assert_eq!(
            DesktopEnvironment::parse("X-Generic:XFCE"),
            DesktopEnvironment::Xfce
        );
        assert_eq!(
            DesktopEnvironment::parse("KDE;Plasma"),
            DesktopEnvironment::Kde
        );
    }
}
