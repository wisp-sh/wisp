"""Loopback-only Cargo sparse and npm registries for executable acceptance tests."""

import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
from urllib.parse import unquote, urlsplit


def index_path(name):
    if len(name) == 1:
        return f"1/{name}"
    if len(name) == 2:
        return f"2/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


class Registries:
    def __init__(self, release_dir, cargo_metadata):
        self.release_dir = Path(release_dir)
        release = json.loads((self.release_dir / "library-release.json").read_text())
        self.cargo_name = release["packages"]["rust"]["name"]
        self.npm_name = release["packages"]["node"]["name"]
        self.version = release["version"]
        self.requests = []
        package = next(p for p in cargo_metadata["packages"] if p["name"] == self.cargo_name)
        self.deps = [{
            "name": d["rename"] or d["name"], "req": d["req"], "features": d["features"],
            "optional": d["optional"], "default_features": d["uses_default_features"],
            "target": d["target"], "kind": d["kind"] or "normal",
            "registry": d["registry"] or "https://github.com/rust-lang/crates.io-index",
            **({"package": d["name"]} if d["rename"] else {}),
        } for d in package["dependencies"]]
        registries = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                path = unquote(urlsplit(self.path).path)
                registries.requests.append(path)
                body, content_type = registries.response(path)
                self.send_response(200 if body is not None else 404)
                if body is None:
                    body = b"not found"
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def response(self, path):
        cargo_bytes = (self.release_dir / "rust.crate").read_bytes()
        npm_bytes = (self.release_dir / "node.tgz").read_bytes()
        if path == "/cargo/config.json":
            data = {"dl": self.url + "/crates/{crate}/{version}/download"}
        elif path == "/cargo/" + index_path(self.cargo_name):
            data = {"name": self.cargo_name, "vers": self.version, "deps": self.deps,
                    "cksum": hashlib.sha256(cargo_bytes).hexdigest(), "features": {}, "yanked": False}
        elif path == f"/crates/{self.cargo_name}/{self.version}/download":
            return cargo_bytes, "application/octet-stream"
        elif path == "/npm/" + self.npm_name:
            manifest = json.loads((self.release_dir / "node/package.json").read_text())
            manifest["dist"] = {
                "tarball": self.url + "/npm/package.tgz",
                "integrity": "sha512-" + base64.b64encode(hashlib.sha512(npm_bytes).digest()).decode(),
                "shasum": hashlib.sha1(npm_bytes).hexdigest(),
            }
            data = {"name": self.npm_name, "dist-tags": {"latest": self.version},
                    "versions": {self.version: manifest}}
        elif path == "/npm/package.tgz":
            return npm_bytes, "application/octet-stream"
        else:
            return None, "text/plain"
        return (json.dumps(data) + "\n").encode(), "application/json"

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
