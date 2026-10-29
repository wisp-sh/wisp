"""Record a build with Wisp from a fresh OIDC-enabled runner.

Only this trusted script runs in the record job. No archive is extracted and
no dependency, package, runtime, or source from the build job is executed.
The files stay in the run's GitHub artifact; Wisp receives their hashes and
downloads the artifact itself when the build is first needed.
"""

import argparse
import os
import re
import sys
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit
from urllib.request import Request

sys.path.insert(0, str(Path(__file__).resolve().parent))
from build import go_module, repository_path, validate_targets
from common import FILE, HTTP, MAX_FILE, MAX_MANIFEST, NAME, claim_registry_names, fail, https_url, json_bytes, read_file, sha256, strict_json, wisp_origin
from python_wheel import wheel_filename

ALLOWED_FILES = {"component.wasm", "library-release.json", "rust.crate", "node.tgz", "cargo.crate", "npm.tgz", "go.zip", "go.mod", "go.info"}
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?\Z")


def required_files(targets, names):
    """Each declared target must ship exactly the archive its registry needs."""
    expected = {"npm": {"npm.tgz"}, "cargo": {"cargo.crate"}, "go": {"go.zip", "go.mod", "go.info"}}
    for registry, files in expected.items():
        present = files & names
        if registry in targets and present != files or registry not in targets and present:
            fail(f"{registry} files do not match the declared targets")
    wheels = {name for name in names if name.endswith(".whl")}
    if ("pypi" in targets) != bool(wheels) or len(wheels) > 1:
        fail("a PyPI target requires exactly one wheel")
    if not {"component.wasm", "library-release.json"} <= names:
        fail("every package needs its component and release record")


def load_artifacts(directory):
    manifest = strict_json(read_file(directory / "manifest.json", MAX_MANIFEST))
    if not isinstance(manifest, dict) or not {"schema", "repository", "packages"} <= set(manifest) or set(manifest) - {"schema", "repository", "packages", "runtimeSha256"} or type(manifest["schema"]) is not int or manifest["schema"] != 2:
        fail("invalid artifact manifest schema")
    repository = repository_path(manifest["repository"])
    if repository != manifest["repository"]:
        fail("artifact manifest repository is not canonical")
    if "runtimeSha256" in manifest and (not isinstance(manifest["runtimeSha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", manifest["runtimeSha256"])):
        fail("invalid runtime digest")
    packages = manifest["packages"]
    if not isinstance(packages, list) or not 1 <= len(packages) <= 32:
        fail("artifact manifest must contain 1 to 32 packages")
    files = {}
    names = set()
    registry_names = set()
    wheel_names = set()
    for package in packages:
        if not isinstance(package, dict) or not {"name", "path", "version", "targets", "files"} <= set(package) or set(package) - {"name", "path", "version", "targets", "files", "summary"}:
            fail("invalid artifact package fields")
        name = package["name"]
        if not isinstance(name, str) or len(name) > 48 or not NAME.fullmatch(name) or name in names:
            fail("invalid or duplicate artifact package name")
        names.add(name)
        if not isinstance(package["path"], str) or not 0 < len(package["path"]) <= 256:
            fail("invalid artifact package path")
        if "summary" in package and (not isinstance(package["summary"], str) or len(package["summary"]) > 280):
            fail("invalid artifact package summary")
        version = package["version"]
        if not isinstance(version, str) or not VERSION.fullmatch(version):
            fail("invalid artifact package version")
        targets = package["targets"]
        if not isinstance(targets, dict):
            fail("invalid artifact targets")
        go = targets.get("go")
        if go is not None and go != go_module(repository, name, version):
            fail("Go module path does not belong to this repository and version")
        validate_targets({**targets, **({"go": True} if go else {})})
        claim_registry_names(targets, registry_names)
        if not isinstance(package["files"], list) or not 2 <= len(package["files"]) <= 16:
            fail("invalid package file count")
        folder = directory / "packages" / name
        if folder.is_symlink() or folder.parent.is_symlink():
            fail("artifact directories cannot be symlinks")
        declared = set()
        for item in package["files"]:
            if not isinstance(item, dict) or set(item) != {"name", "sha256", "size", "mediaType"}:
                fail("invalid artifact file fields")
            filename = item["name"]
            if not isinstance(filename, str) or not FILE.fullmatch(filename) or (filename not in ALLOWED_FILES and not filename.endswith(".whl")):
                fail("invalid artifact filename")
            if filename.endswith(".whl"):
                if filename in wheel_names:
                    fail(f"duplicate wheel filename: {filename}")
                wheel_names.add(filename)
                if "pypi" not in targets or filename != wheel_filename(targets["pypi"], version):
                    fail("wheel filename differs from the declared PyPI name or version")
            key = (name, filename)
            if key in files:
                fail("duplicate artifact file")
            declared.add(filename)
            if type(item["size"]) is not int or not 0 < item["size"] <= MAX_FILE or not isinstance(item["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"]):
                fail("invalid artifact size or digest")
            if not isinstance(item["mediaType"], str) or not re.fullmatch(r"[a-z0-9.+-]+/[a-z0-9.+-]+", item["mediaType"]):
                fail("invalid artifact media type")
            data = read_file(folder / filename)
            if len(data) != item["size"] or sha256(data) != item["sha256"]:
                fail("artifact size or SHA-256 mismatch")
            files[key] = (folder / filename, item)
        required_files(targets, declared)
    expected = {directory / "manifest.json"} | {path for path, _ in files.values()}
    found = set()
    for parent, directories, filenames in os.walk(directory, followlinks=False):
        if any((Path(parent) / child).is_symlink() for child in directories + filenames):
            fail("artifact tree cannot contain symlinks")
        found.update(Path(parent) / child for child in filenames)
    if expected != found:
        fail("artifact tree contains undeclared files")
    return manifest, files


def oidc_token():
    url = https_url(os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"])
    parts = urlsplit(url)
    if not parts.hostname.endswith(".actions.githubusercontent.com"):
        fail("unexpected GitHub OIDC token endpoint")
    query = [(key, value) for key, value in parse_qsl(parts.query) if key != "audience"]
    query.append(("audience", wisp_origin()))
    url = urlunsplit((parts.scheme, parts.netloc, parts.path, urlencode(query), ""))
    request = Request(url, headers={"Authorization": "Bearer " + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]})
    with HTTP.open(request, timeout=30) as response:
        result = strict_json(response.read(32769))
    token = result.get("value")
    if not isinstance(token, str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,16384}", token):
        fail("GitHub did not issue an OIDC token")
    return token


def post(path, data):
    token = oidc_token()
    request = Request(wisp_origin() + path, data=json_bytes(data), method="POST", headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
    with HTTP.open(request, timeout=90) as response:
        data = response.read(MAX_MANIFEST + 1)
    if len(data) > MAX_MANIFEST:
        fail("Wisp response exceeds limit")
    return strict_json(data)


def record(directory, artifact_id, artifact_digest):
    manifest, _ = load_artifacts(directory)
    if not re.fullmatch(r"[1-9][0-9]{0,19}", artifact_id or ""):
        fail("invalid GitHub artifact ID")
    digest = (artifact_digest or "").removeprefix("sha256:")
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        fail("invalid GitHub artifact digest")
    result = post("/api/builds/record", {**manifest, "artifact": {"id": artifact_id, "sha256": digest}})
    build_id = result.get("buildId")
    if not isinstance(build_id, str) or not re.fullmatch(r"[a-zA-Z0-9_-]{1,128}", build_id):
        fail("invalid recorded build ID")
    print(f"Wisp recorded build {build_id}")
    for page in result.get("pages", []):
        if isinstance(page, str) and page.startswith(wisp_origin() + "/"):
            print(page)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", required=True, type=Path)
    args = parser.parse_args()
    if args.artifacts.is_symlink():
        fail("artifact root cannot be a symlink")
    record(args.artifacts.resolve(), os.environ.get("WISP_ARTIFACT_ID", ""), os.environ.get("WISP_ARTIFACT_DIGEST", ""))


if __name__ == "__main__":
    try:
        main()
    except HTTPError as error:
        raise SystemExit(f"Wisp record request failed with HTTP {error.code}")
    except (ValueError, OSError, KeyError) as error:
        raise SystemExit(str(error))
