"""Publish release archives under the owner's registry names.

This runs in the owner's own workflow run with GitHub trusted publishing, so
the registries see the owner's repository and workflow. Wisp holds no
registry credentials. Only inert archives from the build artifact are
uploaded; nothing from them is executed.
"""

import argparse
import base64
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import Request

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import HTTP, fail, read_file, sha256
from record import load_artifacts

USER_AGENT = "Wisp publish action (+https://wisp.codes)"


def registry_json(url):
    """Return parsed JSON, or None when the registry has no such version."""
    try:
        with HTTP.open(Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/json"}), timeout=60) as response:
            return json.loads(response.read(64 * 1024 * 1024))
    except HTTPError as error:
        if error.code == 404:
            return None
        raise


def packages(directory, registry):
    manifest, files = load_artifacts(directory)
    for package in manifest["packages"]:
        if registry in package["targets"]:
            yield package, files


def plan(directory, wheels, output):
    manifest, files = load_artifacts(directory)
    selected = {registry: any(registry in package["targets"] for package in manifest["packages"]) for registry in ["npm", "cargo", "pypi"]}
    wheels.mkdir(parents=True, exist_ok=False)
    for (name, filename), (path, _) in files.items():
        if filename.endswith(".whl"):
            shutil.copyfile(path, wheels / filename)
    with open(output, "a", encoding="utf-8") as handle:
        for registry, value in selected.items():
            handle.write(f"{registry}={'true' if value else 'false'}\n")
    return selected


def publish_npm(directory):
    for package, files in packages(directory, "npm"):
        name, version = package["targets"]["npm"], package["version"]
        path, descriptor = files[(package["name"], "npm.tgz")]
        data = read_file(path)
        integrity = "sha512-" + base64.b64encode(hashlib.sha512(data).digest()).decode()
        existing = registry_json(f"https://registry.npmjs.org/{quote(name, safe='@')}/{quote(version)}")
        if existing is not None:
            if existing.get("dist", {}).get("integrity") != integrity:
                fail(f"npm already has {name}@{version} with different bytes")
            print(f"npm already has {name}@{version}")
            continue
        tag = ["--tag", "next"] if "-" in version else []
        # npm uses GitHub OIDC trusted publishing and attaches provenance.
        subprocess.run(["npm", "publish", str(path), "--access", "public", "--provenance", *tag], check=True)
        print(f"Published {name}@{version} to npm ({descriptor['sha256']})")


def cargo_upload_body(descriptor, archive):
    """Build crates.io's publish request from the crate's normalized manifest."""
    package = descriptor["package"]
    dependencies = []

    def add(table, target):
        for section, kind in [("dependencies", "normal"), ("build-dependencies", "build"), ("dev-dependencies", "dev")]:
            for alias, value in table.get(section, {}).items():
                entry = {"version": value} if isinstance(value, str) else value
                if not entry.get("version") or entry.get("git") or entry.get("path") or entry.get("registry"):
                    fail("Cargo dependency is not a versioned crates.io dependency")
                dependencies.append({"name": entry.get("package", alias), "version_req": entry["version"], "features": entry.get("features", []),
                                     "optional": entry.get("optional", False), "default_features": entry.get("default-features", True),
                                     "target": target, "kind": kind, "registry": "https://github.com/rust-lang/crates.io-index",
                                     "explicit_name_in_toml": alias if "package" in entry else None})
    add(descriptor, None)
    for target, table in descriptor.get("target", {}).items():
        add(table, target)
    metadata = {"name": package["name"], "vers": package["version"], "deps": dependencies, "features": descriptor.get("features", {}),
                "authors": package.get("authors", []), "keywords": package.get("keywords", []), "categories": package.get("categories", []), "badges": {}}
    for key, destination in [("description", "description"), ("documentation", "documentation"), ("homepage", "homepage"), ("license", "license"),
                             ("license-file", "license_file"), ("repository", "repository"), ("links", "links"), ("rust-version", "rust_version")]:
        metadata[destination] = package.get(key)
    encoded = json.dumps(metadata, separators=(",", ":")).encode()
    return struct.pack("<I", len(encoded)) + encoded + struct.pack("<I", len(archive)) + archive


def crate_descriptor(path, name, version):
    with tarfile.open(path, "r:gz") as archive:
        member = archive.getmember(f"{name}-{version}/Cargo.toml")
        if not member.isfile() or member.size > 1024 * 1024:
            fail("Cargo descriptor is missing or too large")
        descriptor = tomllib.loads(archive.extractfile(member).read().decode())
    if descriptor.get("package", {}).get("name") != name or descriptor["package"].get("version") != version:
        fail("Cargo descriptor differs from the declared package")
    if not descriptor["package"].get("license") and not descriptor["package"].get("license-file"):
        fail("crates.io requires an explicit license or license-file")
    return descriptor


def publish_cargo(directory):
    token = os.environ.get("CARGO_REGISTRY_TOKEN", "")
    if not token:
        fail("crates.io trusted publishing did not provide a token")
    for package, files in packages(directory, "cargo"):
        name, version = package["targets"]["cargo"], package["version"]
        path, descriptor = files[(package["name"], "cargo.crate")]
        existing = registry_json(f"https://crates.io/api/v1/crates/{quote(name)}/{quote(version)}")
        if existing is not None:
            if existing.get("version", {}).get("checksum") != descriptor["sha256"]:
                fail(f"crates.io already has {name}@{version} with different bytes")
            print(f"crates.io already has {name}@{version}")
            continue
        data = read_file(path)
        if sha256(data) != descriptor["sha256"]:
            fail("crate changed after verification")
        body = cargo_upload_body(crate_descriptor(path, name, version), data)
        request = Request("https://crates.io/api/v1/crates/new", data=body, method="PUT",
                          headers={"Authorization": token, "Content-Type": "application/octet-stream", "User-Agent": USER_AGENT})
        with HTTP.open(request, timeout=180) as response:
            response.read(1024 * 1024)
        print(f"Published {name}@{version} to crates.io ({descriptor['sha256']})")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["plan", "npm", "cargo"])
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--wheels", type=Path)
    args = parser.parse_args()
    if args.artifacts.is_symlink():
        fail("artifact root cannot be a symlink")
    directory = args.artifacts.resolve()
    if args.command == "plan":
        if not args.wheels:
            fail("plan requires --wheels")
        plan(directory, args.wheels, os.environ["GITHUB_OUTPUT"])
    elif args.command == "npm":
        publish_npm(directory)
    else:
        publish_cargo(directory)


if __name__ == "__main__":
    try:
        main()
    except HTTPError as error:
        raise SystemExit(f"Registry request failed with HTTP {error.code}")
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError, tarfile.TarError) as error:
        raise SystemExit(str(error))
