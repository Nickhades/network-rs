#!/usr/bin/env python3
"""Generate CARGO_CRATES for the FreeBSD port from Cargo.lock."""
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
lock = (root / "Cargo.lock").read_text()
entries = []
for block in lock.split("[[package]]")[1:]:
    name = re.search(r'^name = "([^"]+)"', block, re.M)
    version = re.search(r'^version = "([^"]+)"', block, re.M)
    source = re.search(r'^source = "([^"]+)"', block, re.M)
    if not (name and version and source):
        continue
    if "registry+" not in source.group(1):
        continue
    if name.group(1) in {"nm-common", "nm-freebsd-daemon", "nm-xfce-plugin"}:
        continue
    entries.append(f"{name.group(1)}-{version.group(1)}")

entries = sorted(set(entries))
print("CARGO_CRATES=\t" + " \\\n\t".join(entries))
print(f"# generated from Cargo.lock: {len(entries)} crates", file=sys.stderr)
