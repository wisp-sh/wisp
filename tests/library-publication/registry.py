"""Disposable Cargo/npm/OCI publication endpoints, with immutable versions."""

import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
from urllib.parse import parse_qs, unquote, urlsplit


def index_path(name):
    if len(name) < 3:
        return f"{len(name)}/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


class Registry:
    def __init__(self):
        self.requests = []
        self.crates = {}
        self.npm = {}
        self.npm_tarballs = {}
        self.blobs = {}
        self.manifests = {}
        self.fail_npm_once = False
        self.upload_location_override = None
        registry = self

        class Handler(BaseHTTPRequestHandler):
            def process(self):
                parsed = urlsplit(self.path)
                path = unquote(parsed.path)
                registry.requests.append((self.command, path))
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                try:
                    code, payload, headers = registry.handle(
                        self.command, path, parse_qs(parsed.query), body, self.headers.get("Authorization"))
                except Exception as error:
                    code, payload, headers = 400, {"error": str(error)}, {}
                if not isinstance(payload, bytes):
                    payload = json.dumps(payload).encode()
                    headers.setdefault("Content-Type", "application/json")
                self.send_response(code)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            do_GET = do_POST = do_PUT = process

            def log_message(self, *_args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def handle(self, method, path, query, body, authorization):
        if method != "GET":
            expected = "test-token" if path == "/api/v1/crates/new" else "Bearer test-token"
            if authorization != expected:
                return 401, {}, {}
        if method == "GET":
            if path == "/cargo/config.json":
                return 200, {"api": self.url, "dl": self.url + "/crates/{crate}/{version}/download"}, {}
            for (name, version), (metadata, archive) in self.crates.items():
                if path == "/cargo/" + index_path(name):
                    deps = [{"name": dep.get("explicit_name_in_toml") or dep["name"], "req": dep["version_req"],
                             "features": dep["features"], "optional": dep["optional"],
                             "default_features": dep["default_features"], "target": dep["target"],
                             "kind": dep["kind"], "registry": dep["registry"],
                             **({"package": dep["name"]} if dep.get("explicit_name_in_toml") else {})}
                            for dep in metadata["deps"]]
                    return 200, {"name": name, "vers": version, "deps": deps, "features": metadata["features"],
                                 "cksum": hashlib.sha256(archive).hexdigest(), "yanked": False}, {}
                if path == f"/crates/{name}/{version}/download":
                    return 200, archive, {}
            if path.startswith("/npm/") and path[5:] in self.npm:
                return 200, self.npm[path[5:]], {}
            if path in self.npm_tarballs:
                return 200, self.npm_tarballs[path], {}
            if "/blobs/sha256:" in path and path.rsplit("/", 1)[1] in self.blobs:
                digest = path.rsplit("/", 1)[1]
                return 200, self.blobs[digest], {"Docker-Content-Digest": digest}
            if "/manifests/sha256:" in path and path.rsplit("/", 1)[1] in self.manifests:
                digest = path.rsplit("/", 1)[1]
                return 200, self.manifests[digest], {"Docker-Content-Digest": digest}
            return 404, {}, {}
        if method == "PUT" and path == "/api/v1/crates/new":
            length = int.from_bytes(body[:4], "little")
            metadata = json.loads(body[4:4 + length])
            offset = 4 + length
            archive_length = int.from_bytes(body[offset:offset + 4], "little")
            archive = body[offset + 4:]
            assert len(archive) == archive_length
            key = (metadata["name"], metadata["vers"])
            if key in self.crates:
                return 409, {}, {}
            self.crates[key] = (metadata, archive)
            return 200, {"warnings": {}}, {}
        if method == "PUT" and path.startswith("/npm/"):
            if self.fail_npm_once:
                self.fail_npm_once = False
                return 503, {}, {}
            metadata = json.loads(body)
            name = metadata["name"]
            assert path == "/npm/" + name
            current = self.npm.setdefault(name, {"name": name, "versions": {}, "dist-tags": {}})
            if any(version in current["versions"] for version in metadata["versions"]):
                return 409, {}, {}
            for version, manifest in metadata["versions"].items():
                attachment = metadata["_attachments"][f"{name}-{version}.tgz"]
                archive = base64.b64decode(attachment["data"], validate=True)
                assert attachment["length"] == len(archive)
                assert manifest["dist"]["shasum"] == hashlib.sha1(archive).hexdigest()
                assert manifest["dist"]["integrity"] == "sha512-" + base64.b64encode(hashlib.sha512(archive).digest()).decode()
                self.npm_tarballs[urlsplit(manifest["dist"]["tarball"]).path] = archive
                current["versions"][version] = manifest
            current["dist-tags"].update(metadata["dist-tags"])
            return 201, {"ok": True}, {}
        if method == "POST" and path.endswith("/blobs/uploads/"):
            return 202, b"", {"Location": self.upload_location_override or "session"}
        if method == "PUT" and path.endswith("/blobs/uploads/session"):
            digest = query["digest"][0]
            assert digest == "sha256:" + hashlib.sha256(body).hexdigest()
            if digest in self.blobs and self.blobs[digest] != body:
                return 409, {}, {}
            self.blobs[digest] = body
            return 201, b"", {"Docker-Content-Digest": digest}
        if method == "PUT" and "/manifests/sha256:" in path:
            digest = path.rsplit("/", 1)[1]
            assert digest == "sha256:" + hashlib.sha256(body).hexdigest()
            manifest = json.loads(body)
            assert all(descriptor["digest"] in self.blobs for descriptor in [manifest["config"]] + manifest["layers"])
            if digest in self.manifests and self.manifests[digest] != body:
                return 409, {}, {}
            self.manifests[digest] = body
            return 201, b"", {"Docker-Content-Digest": digest}
        return 404, {}, {}

    def writes(self):
        return [request for request in self.requests if request[0] != "GET"]

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
