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
from common import HTTP, fail, read_file, registry_identity, sha256, strict_json
from python_wheel import wheel_filename
from record import load_artifacts

USER_AGENT = "Wisp publish action (+https://wisp.codes)"


def registry_json(url):
    """Return parsed JSON, or None when the registry has no such version."""
    try:
        with HTTP.open(Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/json"}), timeout=60) as response:
            data = response.read(64 * 1024 * 1024 + 1)
        if len(data) > 64 * 1024 * 1024:
            fail("registry response exceeds limit")
        value = strict_json(data)
        if not isinstance(value, dict):
            fail("invalid registry response")
        return value
    except HTTPError as error:
        error.close()
        if error.code == 404:
            return None
        raise


def packages(directory, registry):
    manifest, files = load_artifacts(directory)
    for package in manifest["packages"]:
        if registry in package["targets"]:
            yield package, files


def collect_wheels(entries, wheels):
    """Create a fresh upload directory; never replace an existing wheel."""
    names = [path.name for path, _ in entries]
    if len(names) != len(set(names)):
        fail("duplicate wheel filename in publication plan")
    wheels.mkdir(parents=True, exist_ok=False)
    for path, descriptor in entries:
        data = read_file(path)
        if len(data) != descriptor["size"] or sha256(data) != descriptor["sha256"]:
            fail("wheel changed after verification")
        with (wheels / path.name).open("xb") as handle:
            handle.write(data)


def plan(directory, wheels, output):
    manifest, files = load_artifacts(directory)
    selected = {registry: any(registry in package["targets"] for package in manifest["packages"]) for registry in ["npm", "cargo", "pypi"]}
    collect_wheels([entry for (_, filename), entry in files.items() if filename.endswith(".whl")], wheels)
    with open(output, "a", encoding="utf-8") as handle:
        for registry, value in selected.items():
            handle.write(f"{registry}={'true' if value else 'false'}\n")
    return selected


def prepare_pypi(directory, wheels, output):
    """Stage missing wheels for the caller's PyPA action, without credentials.

    PyPI's release JSON lists files under `urls`, not `releases`. Only an
    exact filename with the admitted SHA-256 and size counts as published.
    Metadata versions can differ in spelling, so they do not select files.
    """
    pending = []
    for package, files in packages(directory, "pypi"):
        name, version = package["targets"]["pypi"], package["version"]
        filename = wheel_filename(name, version)
        path, descriptor = files[(package["name"], filename)]
        identity = registry_identity("pypi", name)
        existing = registry_json(f"https://pypi.org/pypi/{quote(identity, safe='')}/{quote(version, safe='')}/json")
        match = None
        if existing is not None:
            entries = existing.get("urls")
            if not isinstance(entries, list):
                fail("invalid PyPI release file list")
            seen = set()
            for entry in entries:
                if not isinstance(entry, dict) or not isinstance(entry.get("filename"), str) or not entry["filename"] or entry["filename"] in seen:
                    fail("invalid or duplicate PyPI release filename")
                seen.add(entry["filename"])
                if entry["filename"] == filename:
                    match = entry
        if match is not None:
            digests = match.get("digests")
            if not isinstance(digests, dict) or digests.get("sha256") != descriptor["sha256"] or type(match.get("size")) is not int or match["size"] != descriptor["size"]:
                fail(f"PyPI already has {filename} with different bytes or missing integrity data")
            if match.get("packagetype") != "bdist_wheel":
                fail(f"PyPI returned an invalid file type for {filename}")
            if match.get("yanked") is not False:
                fail(f"PyPI file {filename} is yanked or has unknown yank status")
            print(f"PyPI already has identical {filename}")
        else:
            pending.append((path, descriptor))
    # Check every package before staging anything or declaring success. The
    # upload action must keep skip-existing disabled to expose upload races.
    collect_wheels(pending, wheels)
    with open(output, "a", encoding="utf-8") as handle:
        handle.write(f"pypi={'true' if pending else 'false'}\n")
    return bool(pending)


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
    parser.add_argument("command", choices=["plan", "npm", "cargo", "pypi"])
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--wheels", type=Path)
    args = parser.parse_args()
    if args.artifacts.is_symlink():
        fail("artifact root cannot be a symlink")
    directory = args.artifacts.resolve()
    if args.command in {"plan", "pypi"}:
        if not args.wheels:
            fail(f"{args.command} requires --wheels")
        prepare = plan if args.command == "plan" else prepare_pypi
        prepare(directory, args.wheels, os.environ["GITHUB_OUTPUT"])
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
