"""Bounded, checksum-pinned downloads for source compiler acceptance."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import time
import urllib.parse
import urllib.request

TOOLS = {"clang", "go", "rust", "llvm", "wasm-tools"}
MAX_MANIFEST = 64 * 1024
MAX_BUNDLE = 2 * 1024**3


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def checksum(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value), "expected a lowercase SHA-256")
    return value


def public_https(url):
    require(isinstance(url, str), "download URL must be a string")
    parsed = urllib.parse.urlsplit(url)
    require(parsed.scheme == "https" and parsed.hostname and not parsed.username and not parsed.password
            and not parsed.query and not parsed.fragment, "downloads require a public HTTPS URL without credentials, query or fragment")
    return url


def strict_json(payload):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, f"duplicate JSON key: {key}")
            result[key] = value
        return result
    return json.loads(payload, object_pairs_hook=pairs,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"invalid JSON constant: {value}")))


def manifest(path, expected):
    path = Path(path)
    checksum(expected)
    require(0 < path.stat().st_size <= MAX_MANIFEST, "source compiler manifest exceeds 64 KiB")
    require(digest(path) == expected, "source compiler manifest checksum mismatch")
    value = strict_json(path.read_bytes())
    # Buildtools releases include provenance and all available compiler bundles.
    # Preserve the legacy five-bundle view used by source acceptance.
    if isinstance(value, dict) and value.get("schema") == 2:
        require(value.get("platform") == "linux-x86_64" and isinstance(value.get("bundles"), list), "unsupported compiler release manifest")
        selected = []
        for entry in value["bundles"]:
            require(isinstance(entry, dict) and isinstance(entry.get("tool"), str), "invalid release compiler entry")
            if entry["tool"] not in TOOLS:
                continue
            require(entry.get("platform") == "wasm32-wasi", "source compilers must be Wasm bundles")
            require(isinstance(entry.get("requires"), list) and all(name in {"wisp:" + tool for tool in TOOLS} for name in entry["requires"]), "unsupported shared compiler dependency")
            require(all(key in entry for key in ("tool", "version", "url", "size", "sha256")), "incomplete release compiler entry")
            selected.append({key: entry[key] for key in ("tool", "version", "url", "size", "sha256")})
        value = {"schema": 1, "platform": value["platform"], "bundles": selected}
    require(isinstance(value, dict) and set(value) == {"schema", "platform", "bundles"}
            and type(value["schema"]) is int and value["schema"] == 1
            and value["platform"] == "linux-x86_64", "unsupported source compiler manifest")
    require(isinstance(value["bundles"], list) and len(value["bundles"]) == len(TOOLS), "all five source compiler bundles are required")
    seen = set()
    for bundle in value["bundles"]:
        require(isinstance(bundle, dict) and set(bundle) == {"tool", "version", "url", "size", "sha256"}, "invalid compiler bundle fields")
        require(isinstance(bundle["tool"], str) and bundle["tool"] in TOOLS and bundle["tool"] not in seen, "unknown or duplicate compiler bundle")
        seen.add(bundle["tool"])
        require(isinstance(bundle["version"], str) and re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", bundle["version"]), "bundle version must be a stable canonical SemVer")
        require(type(bundle["size"]) is int and 0 < bundle["size"] <= MAX_BUNDLE, "invalid compiler bundle size")
        checksum(bundle["sha256"])
        public_https(bundle["url"])
    require(seen == TOOLS, "source compiler bundle set differs")
    return value


def verify_file(path, expected, size=None, limit=MAX_BUNDLE):
    path = Path(path)
    require(path.is_file() and not path.is_symlink(), f"regular input file required: {path}")
    actual_size = path.stat().st_size
    require(0 < actual_size <= limit and (size is None or actual_size == size), f"download size mismatch: {path}")
    require(digest(path) == checksum(expected), f"download checksum mismatch: {path}")
    return path


def download(url, expected, destination, *, size=None, limit=MAX_BUNDLE):
    public_https(url)
    checksum(expected)
    destination = Path(destination)
    if destination.exists():
        return verify_file(destination, expected, size, limit)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".partial")
    started = time.monotonic()
    created = False
    try:
        with urllib.request.urlopen(url, timeout=120) as response, temporary.open("xb") as output:
            created = True
            # Redirects may change the asset host, but may not downgrade HTTPS.
            require(urllib.parse.urlsplit(response.url).scheme == "https", "download redirected away from HTTPS")
            length = response.headers.get("Content-Length")
            if length is not None:
                declared = int(length)
                require(0 < declared <= limit and (size is None or declared == size), "download Content-Length mismatch")
            total = 0
            while block := response.read(1024 * 1024):
                total += len(block)
                require(total <= limit and (size is None or total <= size), "download exceeds declared size")
                require(time.monotonic() - started < 1800, "download exceeded 30 minutes")
                output.write(block)
        verify_file(temporary, expected, size, limit)
        temporary.rename(destination)
    finally:
        if created:
            temporary.unlink(missing_ok=True)
    return destination
