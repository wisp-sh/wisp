#!/usr/bin/env python3
"""Record the exact five bundles to publish for source compiler acceptance."""
import argparse
import json
from pathlib import Path
import zipfile

from acquire import TOOLS, digest, manifest, public_https, require


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle-dir", type=Path, required=True)
    parser.add_argument("--base-url", required=True, help="public HTTPS directory that will contain the five .wisp.zip files")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    base = public_https(args.base_url).rstrip("/")
    bundles = []
    for tool in sorted(TOOLS):
        path = args.bundle_dir / (tool + ".wisp.zip")
        with zipfile.ZipFile(path) as archive:
            info = archive.getinfo("release.json")
            require(info.file_size <= 16 * 1024**2, "toolchain release manifest exceeds admission bound")
            record = json.loads(archive.read(info))
        require(record["schema"] == 4 and record["kind"] == "toolchain" and record["tool"] == tool
                and record["identity"] == "wisp:" + tool and record["platform"] == "wasm32-wasi"
                and record["sizes"]["nativeBytes"] == 0, f"unexpected bundle identity or compiler platform: {path}")
        bundles.append({"tool": tool, "version": record["version"], "url": base + "/" + path.name,
                        "size": path.stat().st_size, "sha256": digest(path)})
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps({"schema": 1, "platform": "linux-x86_64", "bundles": bundles}, indent=2) + "\n")
    expected = digest(args.out)
    manifest(args.out, expected)
    print(f"{expected}  {args.out}")
    print("Publish the manifest and these exact bundles before configuring CI. This command does not upload files.")


if __name__ == "__main__":
    main()
