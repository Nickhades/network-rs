//! SSID helpers shared by the daemon and the panel plugin.

/// Decode the escaping used by wpa_supplicant's textual control interface.
///
/// Non-ASCII and non-printable SSID bytes are printed as literal `\xNN`
/// sequences (for example a UTF-8 Chinese SSID becomes `\xe4\xba\x91...`).
/// A literal backslash is doubled. This is not a general-purpose C escape
/// parser; it only handles the forms wpa_supplicant emits.
pub fn unescape_wpa(value: &[u8]) -> Vec<u8> {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let mut out = Vec::with_capacity(value.len());
    let mut i = 0;
    while i < value.len() {
        if value[i] == b'\\' {
            if i + 3 < value.len()
                && (value[i + 1] == b'x' || value[i + 1] == b'X')
                && let (Some(high), Some(low)) = (hex(value[i + 2]), hex(value[i + 3]))
            {
                out.push(high * 16 + low);
                i += 4;
                continue;
            }
            if i + 1 < value.len() {
                match value[i + 1] {
                    b'\\' => out.push(b'\\'),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'"' => out.push(b'"'),
                    other => {
                        out.push(b'\\');
                        out.push(other);
                    }
                }
                i += 2;
                continue;
            }
        }
        out.push(value[i]);
        i += 1;
    }
    out
}

/// Render an SSID for logs and menus. SSIDs are arbitrary bytes, not UTF-8.
pub fn ssid_to_string(ssid: &[u8]) -> String {
    String::from_utf8_lossy(ssid).into_owned()
}

/// wpa_supplicant accepts SSIDs as hex, which avoids all quoting problems.
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Quote a passphrase for SET_NETWORK. wpa_supplicant's parser understands
/// backslash escapes inside double quotes.
pub fn escape_wpa_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encoding() {
        assert_eq!(hex_encode(b"hello"), "68656c6c6f");
        assert_eq!(hex_encode(&[0, 255]), "00ff");
    }

    #[test]
    fn quoting() {
        assert_eq!(escape_wpa_string("hunter2"), "\"hunter2\"");
        assert_eq!(escape_wpa_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(escape_wpa_string("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn unescapes_wpa_hex_bytes() {
        assert_eq!(
            unescape_wpa(b"\\xe4\\xba\\x91\\xe6\\xb5\\xb7"),
            "云海".as_bytes()
        );
        assert_eq!(unescape_wpa(b"Cafe WiFi"), b"Cafe WiFi");
        assert_eq!(unescape_wpa(b"a\\\\b"), b"a\\b");
    }

    #[test]
    fn lossy_ssid() {
        assert_eq!(ssid_to_string(b"cafe"), "cafe");
        assert_eq!(ssid_to_string(&[0xff]), "\u{fffd}");
    }
}
