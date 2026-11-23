fn main() {
    let mut spec = xfce_panel::build::DesktopSpec::new(
        "nm_freebsd_xfce",
        "NetworkManager FreeBSD",
        "Wi-Fi status and connection menu",
    );
    spec.icon("network-wireless");
    spec.internal(true);
    xfce_panel::build::write_desktop(&spec);
}
