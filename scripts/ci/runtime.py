"""Install the release pinned by this approved action, never caller input."""

import argparse
import os
import posixpath
import re
import shutil
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
from urllib.request import Request

sys.path.insert(0, str(Path(__file__).resolve().parent))
import hashlib
from urllib.request import HTTPRedirectHandler, build_opener

from common import fail, https_url, json_bytes, strict_json


class HTTPSRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        https_url(new_url)
        return super().redirect_request(request, fp, code, message, headers, new_url)


HTTP = build_opener(HTTPSRedirect())

MAX_RUNTIME = 2 * 1024 * 1024 * 1024
MAX_EXPANDED = 24 * 1024 * 1024 * 1024


def validate_catalog(catalog):
    if not isinstance(catalog, dict) or set(catalog) != {"schema", "platform", "runtime", "cli", "env"}:
        fail("invalid CI catalog fields")
    if type(catalog["schema"]) is not int or catalog["schema"] != 1 or catalog["platform"] != "linux-x86_64":
        fail("unsupported CI runtime platform")
    runtime = catalog["runtime"]
    if not isinstance(runtime, dict) or set(runtime) != {"url", "sha256", "size"}:
        fail("invalid CI runtime descriptor")
    https_url(runtime["url"])
    if not re.fullmatch(r"[0-9a-f]{64}", runtime["sha256"]):
        fail("invalid CI runtime digest")
    if type(runtime["size"]) is not int or not 0 < runtime["size"] <= MAX_RUNTIME:
        fail("invalid CI runtime size")
    if catalog["cli"] != {"path": "bin/wisp"} or not isinstance(catalog["env"], dict):
        fail("runtime CLI must be bin/wisp")
    for name, value in catalog["env"].items():
        if not re.fullmatch(r"WISP_[A-Z0-9_]+|CARGO_HOME|CARGO_TARGET_DIR|GOCACHE|GOMODCACHE", name):
            fail("unsupported runtime environment variable")
        relative(value)
    return catalog


def relative(value):
    if not isinstance(value, str) or not value or "\\" in value or any(ord(c) < 32 for c in value):
        fail("invalid runtime path")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in (".", "..", "") for part in value.split("/")):
        fail("runtime paths must be canonical and relative")
    return path


def extract_runtime(archive, destination):
    # Node's npm/JCO launchers use relative file symlinks. Admit those links,
    # but create them only after all regular-file writes have finished.
    total = 0
    seen = set()
    regular = set()
    links = {}
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle:
            path = relative(member.name.rstrip("/"))
            if str(path) in seen or not (member.isfile() or member.isdir() or member.issym()):
                fail("duplicate or unsupported runtime entry")
            seen.add(str(path))
            total += member.size
            if total > MAX_EXPANDED or len(seen) > 200000:
                fail("expanded runtime exceeds limits")
            if member.issym():
                link = member.linkname
                if not link or link.startswith("/") or "\\" in link or any(ord(c) < 32 for c in link):
                    fail("invalid runtime file symlink")
                target_path = posixpath.normpath(posixpath.join(str(path.parent), link))
                relative(target_path)
                links[str(path)] = (link, target_path)
                continue
            target = destination.joinpath(*path.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)
            target.chmod(0o755 if member.mode & 0o111 else 0o644)
            regular.add(str(path))
    for path, (_, target_path) in links.items():
        visited = {path}
        while target_path in links:
            if target_path in visited:
                fail("runtime file symlinks form a cycle")
            visited.add(target_path)
            target_path = links[target_path][1]
        if target_path not in regular:
            fail("runtime symlinks must resolve to archived regular files")
        if any(str(parent) in links for parent in PurePosixPath(path).parents):
            fail("runtime symlink parents cannot be links")
    for path, (link, _) in links.items():
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.symlink_to(link)


def download(url, expected, destination, *, size=None, limit=MAX_RUNTIME):
    """Verify a release asset, including HTTPS-only GitHub asset redirects."""
    https_url(url)
    if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
        fail("expected a lowercase SHA-256")
    digest = hashlib.sha256()
    total = 0
    with HTTP.open(Request(url), timeout=120) as response, Path(destination).open("xb") as output:
        while chunk := response.read(1024 * 1024):
            total += len(chunk)
            if total > limit or (size is not None and total > size):
                fail("download exceeds declared size")
            digest.update(chunk)
            output.write(chunk)
    if not total or (size is not None and total != size) or digest.hexdigest() != expected:
        fail("download size or SHA-256 mismatch")
    return Path(destination)


DEFAULT_LOCK = Path(__file__).with_name("runtime.lock.json")


def load_catalog(lock_path, destination):
    data = Path(lock_path).read_bytes()
    if len(data) > 65536:
        fail("runtime pin exceeds 64 KiB")
    lock = strict_json(data)
    if not isinstance(lock, dict) or set(lock) != {"schema", "manifestUrl", "manifestSha256"} or type(lock["schema"]) is not int or lock["schema"] != 1:
        fail("invalid approved runtime pin")
    if lock["manifestUrl"] is None or lock["manifestSha256"] is None:
        fail("No runtime release is pinned yet. Publish the CLI kit and buildtools runtime, then update scripts/ci/runtime.lock.json in the approved action.")
    saved = download(lock["manifestUrl"], lock["manifestSha256"], destination, limit=65536)
    return validate_catalog(strict_json(saved.read_bytes()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    if args.out.exists():
        fail("runtime destination must be fresh")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="wisp-runtime-", dir=args.out.parent) as temporary:
        root = Path(temporary)
        catalog = load_catalog(DEFAULT_LOCK, root / "manifest.json")
        descriptor = catalog["runtime"]
        archive = download(descriptor["url"], descriptor["sha256"], root / "runtime.tar.gz", size=descriptor["size"])
        stage = root / "installed"
        stage.mkdir()
        extract_runtime(archive, stage)
        for name in ("wisp", "wisp-worker"):
            if not (stage / "bin" / name).is_file() or not os.access(stage / "bin" / name, os.X_OK):
                fail(f"runtime is missing executable bin/{name}")
        for path in catalog["env"].values():
            if not (stage / path).exists():
                fail(f"runtime environment path is missing: {path}")
        (stage / "wisp-runtime.json").write_bytes(json_bytes(catalog))
        stage.rename(args.out)
    print(f"Installed Wisp build runtime {descriptor['sha256']}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        raise SystemExit(str(error))
