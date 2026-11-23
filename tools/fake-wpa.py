#!/usr/bin/env python3
"""Minimal datagram wpa_supplicant control-interface fake for UI tests."""
import os
import socket
import sys
import time

path = sys.argv[1]
connected = "--disconnected" not in sys.argv
alternate_strength = os.environ.get("FAKE_WPA_ALTERNATE") == "1"
scan_count = 0

parent = os.path.dirname(path)
if parent:
    os.makedirs(parent, exist_ok=True)
try:
    os.unlink(path)
except FileNotFoundError:
    pass

srv = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
srv.bind(path)
os.chmod(path, 0o666)
print("fake-wpa listening on", path, flush=True)

SCAN = (
    "bssid / frequency / signal level / flags / ssid\n"
    "aa:bb:cc:dd:ee:01\t2412\t-42\t[WPA2-PSK-CCMP][ESS]\tCafe WiFi\n"
    "aa:bb:cc:dd:ee:02\t5180\t-77\t[ESS]\tOpen Net\n"
    "aa:bb:cc:dd:ee:03\t2437\t-63\t[WPA2-PSK-CCMP][ESS]\tLab 5G\n"
)

def response(cmd):
    global connected, scan_count
    if cmd == "ATTACH":
        return b"OK\n"
    if cmd == "SCAN":
        return b"OK\n"
    if cmd == "SCAN_RESULTS":
        scan_count += 1
        if alternate_strength and scan_count % 2 == 0:
            return SCAN.replace("\t-42\t", "\t-70\t").encode()
        return SCAN.encode()
    if cmd == "STATUS":
        if connected:
            return (
                b"bssid=aa:bb:cc:dd:ee:01\n"
                b"ssid=Cafe WiFi\n"
                b"wpa_state=COMPLETED\n"
                b"key_mgmt=WPA2-PSK\n"
            )
        return b"wpa_state=DISCONNECTED\n"
    if cmd == "ADD_NETWORK":
        return b"0\n"
    if cmd.startswith("SELECT_NETWORK"):
        connected = True
        return b"OK\n"
    if cmd == "DISCONNECT":
        connected = False
        return b"OK\n"
    if cmd.startswith("LIST_NETWORKS"):
        return b"network id / ssid / bssid / flags\n0\tCafe WiFi\tany\t[CURRENT]\n"
    return b"OK\n"

while True:
    data, addr = srv.recvfrom(8192)
    cmd = data.decode("utf-8", "replace").strip()
    if not cmd:
        continue
    print("fake-wpa <-", cmd, flush=True)
    try:
        srv.sendto(response(cmd), addr)
    except OSError as exc:
        print("fake-wpa reply failed:", exc, flush=True)
