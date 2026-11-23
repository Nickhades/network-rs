# Hardware and band support

This project is a NetworkManager-compatible control plane. It can only manage
interfaces and access points that the FreeBSD kernel driver already exposes.
It is not a wireless driver and cannot add channels, bands, or firmware support.

## Why an SSID or band can be missing

An 802.11 SSID is a byte string (0-32 octets), not a Unicode string. Chinese
names are UTF-8 bytes; `wpa_cli` prints them as `\xNN` escapes and
`ifconfig` may print a short hex form. The daemon keeps the raw bytes and the
CLI renders them as UTF-8. Hex output is not a lost network.

A band or SSID is absent from `nm-freebsd-cli wifi list` when:

- the driver does not expose that channel/band;
- the AP is on a channel the current regulatory/driver channel list omits;
- the AP is hidden, out of range, or not beaconing;
- the adapter/firmware does not support that mode (for example 5GHz VHT).

`nm-freebsd-cli doctor` prints both the channels reported by
`ifconfig <iface> list chan` and the bands visible in the current scan. That
distinction is intentional: "supported by the radio" and "visible right now"
are different facts.

## Support matrix (guidance, not a guarantee)

Support depends on the FreeBSD release, firmware package, regulatory domain,
and the specific chipset. Always verify with:

```sh
ifconfig <iface> list chan
nm-freebsd-cli doctor
```

| Driver / family | Typical bands | Status in this project |
| --- | --- | --- |
| `iwlwifi` (Intel) | 2.4 / 5GHz; 6GHz on supported chipsets | Recommended where the FreeBSD driver/firmware supports the chipset |
| `ath`, `ath10k`, `ath11k` | 2.4 / 5GHz; 6GHz varies | Usable when the FreeBSD driver exposes the interface |
| `urtwn`, `run`, `rtwn` (USB) | Mostly 2.4GHz; some 5GHz | Check the exact USB ID and driver |
| `rtw88`, `rtw89` | 2.4GHz; limited 5GHz | Experimental; known stability issues, including kernel panics |
| Ethernet (`em`, `igb`, `re`, USB Ethernet) | Wired only | Fully managed by the daemon |

This table is not a compatibility promise. A chipset can be supported by
FreeBSD but still be limited by firmware, country code, or antenna design.

### rtw89 in this test environment

The current RTL8852BE test machine exposes only 2.4GHz channels plus
149/153/157/161 in 5GHz, with:

```text
compat.linuxkpi.rtw89_support_ht  = 0
compat.linuxkpi.rtw89_support_vht = 0
compat.linuxkpi.rtw89_support_eht = 0
```

It has produced kernel page faults under repeated association. Do not present
rtw89 as a reliable 5GHz solution for downstream users.

## Distribution guidance

- Ship a support matrix, not a claim of universal Wi-Fi support.
- Detect interfaces at runtime; never hard-code `wlan0` only.
- Report unsupported bands explicitly instead of silently hiding networks.
- Keep the daemon usable with Ethernet and USB tethering when Wi-Fi is limited.
- Treat hidden SSIDs as an explicit profile feature; a normal scan cannot list
  a non-beaconing network.
- Test on the FreeBSD release and firmware users will actually run.

## What the project can and cannot do

Can:

- enumerate `wlan*` and Ethernet interfaces;
- scan, connect, and report APs the driver returns;
- show per-SSID bands and BSSID counts;
- diagnose supported channels versus visible BSSes;
- manage DHCP and addresses on interfaces that exist.

Cannot:

- make a driver scan a channel it does not expose;
- turn a 2.4GHz-only adapter into a 5GHz adapter;
- display a hidden SSID from a normal scan;
- guarantee that a particular chipset is stable under FreeBSD.
