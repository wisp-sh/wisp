"""Build every .wisp.json package with the pinned Wisp runtime."""

import argparse
import gzip
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path, PurePosixPath

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import MAX_FILE, MAX_MANIFEST, NAME, fail, go_host, json_bytes, read_file, sha256, strict_json, wisp_origin
from python_wheel import make_wheel

TARGETS = {"npm", "cargo", "pypi", "go"}
NPM_NAME = re.compile(r"(?:@[a-z0-9][a-z0-9._-]*/)?[a-z0-9][a-z0-9._-]*\Z")
CARGO_NAME = re.compile(r"[A-Za-z][A-Za-z0-9_-]{0,63}\Z")
PYPI_NAME = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9._-]{0,98}[A-Za-z0-9])?\Z")
REPOSITORY = re.compile(r"[a-z0-9][a-z0-9-]{0,38}/[a-z0-9_-][a-z0-9_.-]{0,99}\Z")


def validate_targets(targets):
    """Registry names belong to the repository owner and are never derived by Wisp."""
    if not isinstance(targets, dict) or set(targets) - TARGETS:
        fail("targets may select npm, cargo, pypi, and go")
    for registry, pattern, limit in [("npm", NPM_NAME, 214), ("cargo", CARGO_NAME, 64), ("pypi", PYPI_NAME, 100)]:
        if registry in targets:
            value = targets[registry]
            if not isinstance(value, str) or len(value) > limit or not pattern.fullmatch(value):
                fail(f"targets.{registry} must be a valid {registry} package name")
    if "go" in targets and targets["go"] is not True:
        fail("targets.go must be true when present")


def manifest(source):
    value = strict_json(read_file(source / ".wisp.json", MAX_MANIFEST))
    if isinstance(value, dict) and value.get("schema") == 1:
        fail(".wisp.json schema 1 used Wisp-owned registry names; use schema 2 with your own package names in targets")
    if not isinstance(value, dict) or set(value) != {"schema", "packages"} or type(value["schema"]) is not int or value["schema"] != 2:
        fail(".wisp.json requires schema: 2 and a packages array")
    if not isinstance(value["packages"], list) or not 1 <= len(value["packages"]) <= 32:
        fail("declare between 1 and 32 packages")
    names = set()
    paths = set()
    for package in value["packages"]:
        if not isinstance(package, dict) or set(package) - {"name", "path", "targets", "summary"} or not {"name", "path", "targets"} <= set(package):
            fail("each package needs name, path, targets, and optional summary")
        if "summary" in package and (not isinstance(package["summary"], str) or len(package["summary"]) > 280):
            fail("package summary must be at most 280 characters")
        name = package["name"]
        if not isinstance(name, str) or len(name) > 48 or not NAME.fullmatch(name) or name in names:
            fail("package names must be unique lowercase words separated by hyphens")
        names.add(name)
        path = package["path"]
        if not isinstance(path, str) or not path or "\\" in path or any(ord(c) < 32 for c in path):
            fail("package path is invalid")
        if path != "." and (PurePosixPath(path).is_absolute() or any(p in {"", ".", ".."} for p in path.split("/"))):
            fail("package paths must be canonical paths inside the repository")
        target = (source / path).resolve()
        if not target.is_relative_to(source) or target in paths or not target.is_dir():
            fail("package path must identify a unique directory in the repository")
        paths.add(target)
        validate_targets(package["targets"])
    return value


def repository_path(repository):
    """GitHub names are case-insensitive, so Wisp paths use the lowercase form."""
    value = repository.lower() if isinstance(repository, str) else ""
    if not REPOSITORY.fullmatch(value) or any(part.startswith(".") or part.endswith(".") for part in value.split("/")):
        fail("the GitHub repository name cannot be used in a Wisp package path")
    return value


def go_module(repository, name, version):
    major = int(version.split(".")[0])
    return f"{go_host()}/{repository_path(repository)}/{name}" + (f"/v{major}" if major >= 2 else "")


def verification_url(repository, name, version):
    return f"{wisp_origin()}/packages/{repository_path(repository)}/{name}/{version}"


def archive_files(data):
    files = {}
    expanded = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        for item in archive:
            path = item.name.rstrip("/")
            if not path or PurePosixPath(path).is_absolute() or any(p in {"", ".", ".."} for p in path.split("/")) or "\\" in path:
                fail("unsafe package archive path")
            if not item.isfile() and not item.isdir():
                fail("package archive contains links or special files")
            if item.isdir():
                continue
            expanded += item.size
            if item.size < 0 or item.size > MAX_FILE or expanded > 256 * 1024 * 1024 or len(files) >= 4096 or path in files:
                fail("package archive exceeds limits or contains duplicate files")
            files[path] = archive.extractfile(item).read()
    return files


def write_tar(files, destination):
    with destination.open("xb") as output, gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for path, data in sorted(files.items()):
            item = tarfile.TarInfo(path)
            item.size = len(data)
            item.mode = 0o644
            archive.addfile(item, io.BytesIO(data))


def replace_toml_name(data, section, old, new):
    text = data.decode()
    block = re.compile(r"(?ms)(^\[" + re.escape(section) + r"\]\s*\n)(.*?)(?=^\[|\Z)")
    matches = list(block.finditer(text))
    if len(matches) != 1:
        fail(f"expected one [{section}] descriptor")
    match = matches[0]
    body, count = re.subn(r'(?m)^name\s*=\s*"' + re.escape(old) + r'"\s*$', 'name = "' + new + '"', match[2])
    if count != 1:
        fail("package descriptor name mismatch")
    return (text[:match.start(2)] + body + text[match.end(2):]).encode()


def add_toml_key(data, section, key, value):
    """Add a string key to one TOML table unless the author already set it."""
    text = data.decode()
    block = re.compile(r"(?ms)(^\[" + re.escape(section) + r"\]\s*\n)(.*?)(?=^\[|\Z)")
    matches = list(block.finditer(text))
    if len(matches) != 1:
        fail(f"expected one [{section}] descriptor")
    match = matches[0]
    if re.search(r"(?m)^" + re.escape(key) + r"\s*=", match[2]):
        return data
    line = f"{key} = {json.dumps(value)}\n"
    return (text[:match.end(1)] + line + text[match.end(1):]).encode()


def publication_archives(targets, version, release, folder, url):
    """Rename the CLI's archives to the owner's registry names.

    Each archive gains a link to the Wisp page for this exact version. An
    author's own homepage is preserved.
    """
    node = archive_files(read_file(folder / "node.tgz"))
    descriptor = strict_json(node["package/package.json"])
    if descriptor["name"] != release["packages"]["node"]["name"] or descriptor["version"] != version or descriptor.get("scripts"):
        fail("npm descriptor differs from the admitted Wisp package")
    notices = {path.removeprefix("package/"): data for path, data in node.items()
               if path.startswith("package/notices/") or path == "package/WISP-NOTICES.txt"}
    if "npm" in targets:
        descriptor["name"] = targets["npm"]
        descriptor.setdefault("homepage", url)
        descriptor["wisp"] = {"verification": url}
        node["package/package.json"] = json_bytes(descriptor)
        write_tar(node, folder / "npm.tgz")
    if "cargo" in targets:
        cargo = archive_files(read_file(folder / "rust.crate"))
        old = release["packages"]["rust"]["name"]
        new = targets["cargo"]
        old_root, new_root = f"{old}-{version}/", f"{new}-{version}/"
        if any(not path.startswith(old_root) for path in cargo):
            fail("Cargo archive has an unexpected root")
        cargo = {new_root + path[len(old_root):]: data for path, data in cargo.items()}
        for filename in ["Cargo.toml", "Cargo.toml.orig"]:
            key = new_root + filename
            if tomllib.loads(cargo[key].decode())["package"]["name"] != old:
                fail("Cargo descriptor differs from the admitted Wisp package")
            cargo[key] = replace_toml_name(cargo[key], "package", old, new)
            if filename == "Cargo.toml":
                cargo[key] = replace_toml_name(cargo[key], "lib", old.replace("-", "_"), new.replace("-", "_"))
                cargo[key] = add_toml_key(cargo[key], "package", "homepage", url)
        lock_key = new_root + "Cargo.lock"
        blocks = cargo[lock_key].decode().split("[[package]]")
        changed = 0
        for index in range(1, len(blocks)):
            data = tomllib.loads("[[package]]" + blocks[index])["package"][0]
            if data["name"] == old and data["version"] == version and "source" not in data:
                blocks[index], count = re.subn(r'(?m)^name = "' + re.escape(old) + '"$', 'name = "' + new + '"', blocks[index])
                changed += count
        if changed != 1:
            fail("Cargo lock root does not match the admitted package")
        cargo[lock_key] = "[[package]]".join(blocks).encode()
        write_tar(cargo, folder / "cargo.crate")
    return notices


def media_type(filename):
    if filename.endswith(".wasm"):
        return "application/wasm"
    if filename.endswith(".json"):
        return "application/json"
    if filename.endswith((".zip", ".whl")):
        return "application/zip"
    if filename.endswith((".mod", ".info")):
        return "text/plain"
    return "application/gzip"


def build(source, runtime, output, repository):
    config = manifest(source)
    repository = repository_path(repository)
    catalog = strict_json(read_file(runtime / "wisp-runtime.json", 65536))
    cli = runtime / "bin/wisp"
    environment = os.environ.copy()
    environment.update(CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0")
    bins = [runtime / path for path in ["bin", "rust/bin", "node/bin", "go/bin", "js/node_modules/.bin"]]
    environment["PATH"] = os.pathsep.join(map(str, bins)) + os.pathsep + environment.get("PATH", "")
    for name, relative in catalog["env"].items():
        environment[name] = str(runtime / relative)
    output.mkdir(parents=True, exist_ok=False)
    result = {"schema": 2, "repository": repository, "packages": []}
    if isinstance(catalog.get("runtime"), dict) and re.fullmatch(r"[0-9a-f]{64}", str(catalog["runtime"].get("sha256", ""))):
        result["runtimeSha256"] = catalog["runtime"]["sha256"]
    for package in config["packages"]:
        path = source / package["path"]
        metadata = tomllib.loads(read_file(path / "wisp.toml", MAX_MANIFEST).decode())["package"]
        version = metadata["version"]
        if not isinstance(version, str) or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?", version):
            fail("wisp.toml package version must be canonical SemVer")
        targets = dict(package["targets"])
        folder = output / "packages" / package["name"]
        args = [str(cli), "build", str(path), "--out", str(folder)]
        if targets.get("go"):
            targets["go"] = go_module(repository, package["name"], version)
            args.extend(["--go-module", targets["go"]])
        subprocess.run(args, env=environment, check=True, cwd=source)
        subprocess.run([str(cli), "library", "validate", str(folder)], env=environment, check=True, cwd=source)
        release = strict_json(read_file(folder / "library-release.json", MAX_MANIFEST))
        if release["version"] != version or release["componentSha256"] != sha256(read_file(folder / "component.wasm")):
            fail("Wisp output does not match declared version or Component")
        url = verification_url(repository, package["name"], version)
        notices = publication_archives(targets, version, release, folder, url)
        filenames = ["component.wasm", "library-release.json", "node.tgz", "rust.crate"]
        filenames += [name for registry, name in [("npm", "npm.tgz"), ("cargo", "cargo.crate")] if registry in targets]
        if "go" in targets:
            filenames += ["go.zip", "go.mod", "go.info"]
        if "pypi" in targets:
            wheel = make_wheel(targets["pypi"], version, release, read_file(folder / "component.wasm"), folder, notices, url)
            filenames.append(wheel.name)
        files = []
        for filename in filenames:
            data = read_file(folder / filename)
            files.append({"name": filename, "sha256": sha256(data), "size": len(data), "mediaType": media_type(filename)})
        # Keep only package data. Intermediate tool outputs and source
        # snapshots must never cross into the privileged record job.
        for item in folder.iterdir():
            if item.name not in filenames:
                if item.is_dir() and not item.is_symlink():
                    shutil.rmtree(item)
                else:
                    item.unlink()
        entry = {"name": package["name"], "path": package["path"], "version": version, "targets": targets, "files": files}
        if "summary" in package:
            entry["summary"] = package["summary"]
        result["packages"].append(entry)
    (output / "manifest.json").write_bytes(json_bytes(result))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--repository", required=True, help="GitHub owner/name of the source repository")
    args = parser.parse_args()
    result = build(args.source.resolve(), args.runtime.resolve(), args.out.resolve(), args.repository)
    print(f"Built {len(result['packages'])} Wisp packages")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error))
