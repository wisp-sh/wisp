"""Bounded, strict input readers shared by the approved CI scripts."""

import hashlib
import json
import os
import re
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, build_opener

NAME = re.compile(r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*\Z")
FILE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,199}\Z")
MAX_FILE = 128 * 1024 * 1024
MAX_MANIFEST = 1024 * 1024


def fail(message):
    raise ValueError(message)


def strict_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                fail(f"duplicate JSON key: {key}")
            result[key] = value
        return result
    return json.loads(data, object_pairs_hook=pairs,
                      parse_constant=lambda value: fail(f"invalid JSON: {value}"))


def read_file(path, limit=MAX_FILE):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        fail(f"not a bounded regular file: {path}")
    return path.read_bytes()


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def registry_identity(registry, name):
    """Canonical identities used by the public registries, not display names."""
    if registry == "pypi":
        return re.sub(r"[-_.]+", "-", name).lower()
    if registry == "cargo":
        return name.lower().replace("_", "-")
    return name


def claim_registry_names(targets, seen):
    """A logical package must have its own registry identity at every version."""
    for registry in ("npm", "cargo", "pypi"):
        if registry in targets:
            key = (registry, registry_identity(registry, targets[registry]))
            if key in seen:
                fail(f"duplicate {registry} registry identity: {key[1]}")
            seen.add(key)


def https_url(value):
    if not isinstance(value, str):
        fail("expected an HTTPS URL")
    parsed = urlsplit(value)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.fragment:
        fail("expected HTTPS without credentials or fragment")
    return value


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        fail("redirects are not allowed")


HTTP = build_opener(NoRedirect())


def json_bytes(value):
    return (json.dumps(value, separators=(",", ":"), sort_keys=True) + "\n").encode()


def wisp_origin():
    """The Wisp deployment that records builds. Its origin is also the OIDC audience."""
    value = os.environ.get("WISP_ORIGIN") or "https://wisp.codes"
    parsed = urlsplit(value)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.path not in ("", "/") or parsed.query or parsed.fragment:
        fail("WISP_ORIGIN must be an HTTPS origin")
    return f"https://{parsed.netloc}"


def go_host():
    """The host serving Go modules for that deployment."""
    value = os.environ.get("WISP_GO_HOST") or "go.wisp.codes"
    if not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+", value):
        fail("WISP_GO_HOST must be a lowercase host name")
    return value
