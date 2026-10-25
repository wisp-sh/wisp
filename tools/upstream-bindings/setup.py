#!/usr/bin/env python3
"""Install the pinned upstream binding generator, without compiling a toolchain."""
import argparse
import hashlib
import io
from pathlib import Path
import platform
import subprocess
import tarfile
import urllib.request

VERSION = "0.61.1"
PINS = {
    ("Linux", "x86_64"): ("x86_64-linux", "1aea51f8379e36e76081c08fcc5faada928c03a6d0965a55132d72c8d966463c"),
    ("Linux", "aarch64"): ("aarch64-linux", "f696fe9d1f3ded1926f3648e07371dc5b55467125e57c89ebbe3d3e595c1c1c2"),
    ("Darwin", "arm64"): ("aarch64-macos", "7f3a73a917a73a67d9d9b8e422dc4e938d3bdb749926109426dd631de2298365"),
    ("Darwin", "x86_64"): ("x86_64-macos", "f14010b6531e6fe420763b9e04d000f81f4afb535285c6da5cff49fee585f96c"),
}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    target, digest = PINS[(platform.system(), platform.machine())]
    name = f"wit-bindgen-{VERSION}-{target}"
    url = f"https://github.com/bytecodealliance/wit-bindgen/releases/download/v{VERSION}/{name}.tar.gz"
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read(16 * 1024 * 1024 + 1)
    if len(data) > 16 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != digest:
        raise SystemExit("wit-bindgen release checksum mismatch")
    args.out.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for member in archive:
            path = Path(member.name)
            if len(path.parts) != 2 or path.parts[0] != name or not member.isfile():
                continue
            if member.size > 64 * 1024 * 1024:
                raise SystemExit("wit-bindgen release member too large")
            destination = args.out / path.name
            destination.write_bytes(archive.extractfile(member).read())
            destination.chmod(0o755 if path.name == "wit-bindgen" else 0o644)
    executable = args.out.resolve() / 'wit-bindgen'
    result = subprocess.run([executable, '--version'], text=True, capture_output=True, timeout=30)
    if result.returncode or result.stdout.strip() not in (
            'wit-bindgen-cli 0.61.1', 'wit-bindgen-cli 0.61.1 (0b0069d6c 2026-08-25)'):
        raise SystemExit(f'wit-bindgen release failed its version check: {result.stdout}\n{result.stderr}')
    print(result.stdout.strip())
    print(f"WISP_WIT_BINDGEN={args.out.resolve() / 'wit-bindgen'}")

if __name__ == "__main__":
    main()
