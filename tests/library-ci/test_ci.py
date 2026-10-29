import copy
import importlib.util
import io
import json
import os
import sys
import tarfile
import tempfile
import tomllib
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
from build import archive_files, go_module, manifest, publication_archives, repository_path, write_tar
from common import go_host, json_bytes, sha256, strict_json, wisp_origin
from python_wheel import generate_source, make_wheel
from runtime import extract_runtime, validate_catalog, load_catalog, HTTPSRedirect
from publish import cargo_upload_body, plan
from record import load_artifacts


class ManifestTests(unittest.TestCase):
    def test_duplicate_json_keys_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            strict_json('{"schema":1,"schema":1}')

    def test_monorepo_and_path_boundaries(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            (source / "first").mkdir()
            (source / "second").mkdir()
            packages = [{"name": "first", "path": "first", "targets": {"npm": "@octocat/first", "pypi": "octocat-first"}},
                        {"name": "second", "path": "second", "targets": {"cargo": "octocat-second", "go": True}}]
            (source / ".wisp.json").write_bytes(json_bytes({"schema": 2, "packages": packages}))
            self.assertEqual(len(manifest(source)["packages"]), 2)
            packages[1]["path"] = "../outside"
            (source / ".wisp.json").write_bytes(json_bytes({"schema": 2, "packages": packages}))
            with self.assertRaisesRegex(ValueError, "paths"):
                manifest(source)

    def test_symlink_cannot_escape_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "source"
            source.mkdir()
            (source / "outside").symlink_to(source.parent, target_is_directory=True)
            (source / ".wisp.json").write_bytes(json_bytes({"schema": 2, "packages": [{"name": "first", "path": "outside", "targets": {}}]}))
            with self.assertRaisesRegex(ValueError, "directory"):
                manifest(source)

    def test_registry_names_belong_to_the_owner(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            (source / "pkg").mkdir()
            (source / ".wisp.json").write_bytes(json_bytes({"schema": 1, "packages": [{"name": "pkg", "path": "pkg", "registries": ["npm"]}]}))
            with self.assertRaisesRegex(ValueError, "schema 2"):
                manifest(source)
            for targets in [{"npm": "Not Valid"}, {"cargo": "-bad"}, {"pypi": "bad-"}, {"go": "yes"}, {"maven": "x"}]:
                (source / ".wisp.json").write_bytes(json_bytes({"schema": 2, "packages": [{"name": "pkg", "path": "pkg", "targets": targets}]}))
                with self.subTest(targets=targets), self.assertRaises(ValueError):
                    manifest(source)

    def test_a_test_deployment_can_set_its_origin_and_go_host(self):
        from unittest.mock import patch
        self.assertEqual((wisp_origin(), go_host()), ("https://wisp.codes", "go.wisp.codes"))
        with patch.dict(os.environ, {"WISP_ORIGIN": "https://wisp-test.example.com/", "WISP_GO_HOST": "go.example.com"}):
            self.assertEqual((wisp_origin(), go_host()), ("https://wisp-test.example.com", "go.example.com"))
            self.assertEqual(go_module("octocat/math", "hello", "1.0.0"), "go.example.com/octocat/math/hello")
        for origin in ["http://wisp.codes", "https://wisp.codes/api", "https://user@wisp.codes", "https://wisp.codes?x=1"]:
            with self.subTest(origin=origin), patch.dict(os.environ, {"WISP_ORIGIN": origin}), self.assertRaises(ValueError):
                wisp_origin()
        with patch.dict(os.environ, {"WISP_GO_HOST": "Go.Example.com"}), self.assertRaises(ValueError):
            go_host()

    def test_go_modules_live_under_the_lowercase_repository(self):
        self.assertEqual(go_module("Octocat/Math", "hello", "1.2.0"), "go.wisp.codes/octocat/math/hello")
        self.assertEqual(go_module("octocat/math", "hello", "3.0.0"), "go.wisp.codes/octocat/math/hello/v3")
        for repository in ["octocat/.github", "octocat", "a/b/c", "octocat/math."]:
            with self.subTest(repository=repository), self.assertRaises(ValueError):
                repository_path(repository)


class RuntimeTests(unittest.TestCase):
    def test_unpublished_pin_fails_without_network(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pin = root / "pin.json"
            pin.write_text(json.dumps({"schema": 1, "manifestUrl": None, "manifestSha256": None}))
            with patch("runtime.HTTP.open") as network, self.assertRaisesRegex(ValueError, "No runtime release"):
                load_catalog(pin, root / "manifest.json")
            network.assert_not_called()

    def test_manifest_bytes_are_authenticated_before_parsing(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            pin = root / "pin.json"
            pin.write_text(json.dumps({"schema": 1, "manifestUrl": "https://example.com/release.json", "manifestSha256": "0" * 64}))
            with patch("runtime.HTTP.open") as network:
                network.return_value.__enter__.return_value.read.side_effect = [b"not JSON", b""]
                with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                    load_catalog(pin, root / "manifest.json")

    def test_release_pin_is_independent_of_the_recording_deployment(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            catalog = {"schema": 1, "platform": "linux-x86_64", "cli": {"path": "bin/wisp"}, "env": {},
                       "runtime": {"url": "https://example.com/runtime.tar.gz", "sha256": "a" * 64, "size": 1}}
            data = json_bytes(catalog)
            pin = root / "pin.json"
            url = "https://github.com/wisp-sh/wisp-buildtools/releases/download/runtime-1/runtime-linux-x86_64.json"
            pin.write_bytes(json_bytes({"schema": 1, "manifestUrl": url, "manifestSha256": sha256(data)}))
            with patch.dict(os.environ, {"WISP_ORIGIN": "https://test.example.com"}), patch("runtime.HTTP.open") as network:
                network.return_value.__enter__.return_value.read.side_effect = [data, b""]
                self.assertEqual(load_catalog(pin, root / "manifest.json"), catalog)
                self.assertEqual(network.call_args.args[0].full_url, url)

    def test_release_redirects_must_keep_https(self):
        from urllib.request import Request
        handler = HTTPSRedirect()
        request = Request("https://github.com/owner/repo/releases/download/v1/runtime")
        redirect = handler.redirect_request(request, None, 302, "Found", {}, "https://release-assets.githubusercontent.com/runtime?sig=x")
        self.assertEqual(redirect.full_url, "https://release-assets.githubusercontent.com/runtime?sig=x")
        with self.assertRaises(ValueError):
            handler.redirect_request(request, None, 302, "Found", {}, "http://example.com/runtime")

    def test_catalog_cannot_inject_process_environment(self):
        catalog = {"schema": 1, "platform": "linux-x86_64", "runtime": {"url": "https://storage.example/runtime.tar.gz", "size": 1, "sha256": "a" * 64}, "cli": {"path": "bin/wisp"}, "env": {"WISP_JCO": "js/jco"}}
        validate_catalog(catalog)
        for key, value in [("LD_PRELOAD", "lib.so"), ("WISP_JCO", "../../bin/run"), ("WISP_JCO", "/bin/run")]:
            modified = copy.deepcopy(catalog)
            modified["env"] = {key: value}
            with self.assertRaises(ValueError):
                validate_catalog(modified)

    def test_tar_cannot_write_symlinks_or_parent_paths(self):
        for name, kind in [("../../escape", tarfile.REGTYPE), ("link", tarfile.SYMTYPE)]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                bundle = directory / "runtime.tar.gz"
                with tarfile.open(bundle, "w:gz") as archive:
                    item = tarfile.TarInfo(name)
                    item.type = kind
                    item.linkname = "/tmp/escape"
                    archive.addfile(item)
                output = directory / "out"
                output.mkdir()
                with self.assertRaises(ValueError):
                    extract_runtime(bundle, output)


class ArtifactTests(unittest.TestCase):
    def fixture(self, directory, targets=None):
        targets = {"npm": "@octocat/math"} if targets is None else targets
        folder = directory / "packages/math"
        folder.mkdir(parents=True)
        files = []
        names = ["component.wasm", "library-release.json", "node.tgz", "rust.crate"] + (["npm.tgz"] if "npm" in targets else []) + (["cargo.crate"] if "cargo" in targets else []) + (["octocat_math-0.1.0-py3-none-any.whl"] if "pypi" in targets else [])
        for name in names:
            data = b"test artifact " + name.encode()
            (folder / name).write_bytes(data)
            files.append({"name": name, "size": len(data), "sha256": sha256(data), "mediaType": "application/octet-stream"})
        result = {"schema": 2, "repository": "octocat/math", "packages": [{"name": "math", "path": ".", "version": "0.1.0", "targets": targets, "files": files}]}
        (directory / "manifest.json").write_bytes(json_bytes(result))
        return result

    def test_record_checks_hashes_and_rejects_hidden_executables(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.fixture(directory)
            self.assertEqual(len(load_artifacts(directory)[1]), 5)
            (directory / "packages/math/component.wasm").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "mismatch"):
                load_artifacts(directory)
            self.fixture_after_cleanup(directory)
            (directory / "postinstall.py").write_text('raise RuntimeError("must never execute")')
            with self.assertRaisesRegex(ValueError, "undeclared"):
                load_artifacts(directory)

    def test_record_requires_files_for_each_declared_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            result = self.fixture(directory)
            result["packages"][0]["targets"] = {"npm": "@octocat/math", "cargo": "octocat-math"}
            (directory / "manifest.json").write_bytes(json_bytes(result))
            with self.assertRaisesRegex(ValueError, "cargo files"):
                load_artifacts(directory)
            result["packages"][0]["targets"] = {"npm": "@octocat/math", "go": "go.wisp.codes/victim/repo/math"}
            (directory / "manifest.json").write_bytes(json_bytes(result))
            with self.assertRaisesRegex(ValueError, "Go module path"):
                load_artifacts(directory)

    def test_publish_plan_selects_declared_registries_and_collects_wheels(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "artifacts"
            directory.mkdir()
            self.fixture(directory, {"cargo": "octocat-math", "pypi": "octocat-math"})
            output = Path(temporary) / "output"
            selected = plan(directory, Path(temporary) / "wheels", output)
            self.assertEqual(selected, {"npm": False, "cargo": True, "pypi": True})
            self.assertEqual(output.read_text(), "npm=false\ncargo=true\npypi=true\n")
            self.assertEqual([path.name for path in (Path(temporary) / "wheels").iterdir()], ["octocat_math-0.1.0-py3-none-any.whl"])

    def test_cargo_upload_body_uses_crates_io_framing(self):
        descriptor = {"package": {"name": "octocat-math", "version": "0.1.0", "license": "MIT", "homepage": "https://wisp.codes/packages/octocat/math/math/0.1.0"},
                      "dependencies": {"serde": "1", "renamed": {"package": "real-crate", "version": "2", "optional": True}}}
        body = cargo_upload_body(descriptor, b"crate bytes")
        length = int.from_bytes(body[:4], "little")
        metadata = json.loads(body[4:4 + length])
        self.assertEqual(metadata["name"], "octocat-math")
        self.assertEqual(metadata["homepage"], descriptor["package"]["homepage"])
        self.assertEqual([(d["name"], d["explicit_name_in_toml"]) for d in metadata["deps"]], [("serde", None), ("real-crate", "renamed")])
        self.assertEqual(int.from_bytes(body[4 + length:8 + length], "little"), len(b"crate bytes"))
        self.assertEqual(body[8 + length:], b"crate bytes")
        with self.assertRaisesRegex(ValueError, "crates.io"):
            cargo_upload_body({"package": {"name": "x", "version": "1.0.0"}, "dependencies": {"local": {"path": "../local"}}}, b"")

    def fixture_after_cleanup(self, directory):
        import shutil
        shutil.rmtree(directory / "packages")
        return self.fixture(directory)

    def test_descriptor_repack_uses_owner_names_and_preserves_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            old = "wisp-demo--math"
            cargo_root = old + "-0.1.0/"
            rust = {cargo_root + "Cargo.toml": f'[package]\nname = "{old}"\nversion = "0.1.0"\n[lib]\nname = "{old.replace("-", "_")}"\npath = "src/lib.rs"\n'.encode(),
                    cargo_root + "Cargo.toml.orig": f'[package]\nname = "{old}"\nversion = "0.1.0"\n'.encode(),
                    cargo_root + "Cargo.lock": f'version = 4\n\n[[package]]\nname = "{old}"\nversion = "0.1.0"\n\n[[package]]\nname = "dependency"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n'.encode(),
                    cargo_root + "src/lib.rs": b"pub fn add(a:u32,b:u32)->u32{a+b}",
                    cargo_root + "component.wasm": b"component"}
            node = {"package/package.json": json_bytes({"name": "@demo/math", "version": "0.1.0", "homepage": "https://example.com"}), "package/index.js": b"export function add(a,b){return a+b}", "package/component.wasm": b"component"}
            write_tar(rust, directory / "rust.crate")
            write_tar(node, directory / "node.tgz")
            original = (directory / "rust.crate").read_bytes()
            release = {"packages": {"rust": {"name": old}, "node": {"name": "@demo/math"}}}
            url = "https://wisp.codes/packages/octocat/math/math/0.1.0"
            publication_archives({"npm": "@octocat/math", "cargo": "octocat-math"}, "0.1.0", release, directory, url)
            result = archive_files((directory / "cargo.crate").read_bytes())
            self.assertEqual(result["octocat-math-0.1.0/component.wasm"], b"component")
            self.assertEqual(result["octocat-math-0.1.0/src/lib.rs"], rust[cargo_root + "src/lib.rs"])
            descriptor = tomllib.loads(result["octocat-math-0.1.0/Cargo.toml"].decode())
            self.assertEqual(descriptor["package"]["name"], "octocat-math")
            self.assertEqual(descriptor["package"]["homepage"], url)
            self.assertEqual(descriptor["lib"]["name"], "octocat_math")
            self.assertEqual(tomllib.loads(result["octocat-math-0.1.0/Cargo.lock"].decode())["package"][0]["name"], "octocat-math")
            self.assertEqual((directory / "rust.crate").read_bytes(), original)
            npm = archive_files((directory / "npm.tgz").read_bytes())
            package = strict_json(npm["package/package.json"])
            self.assertEqual(package["name"], "@octocat/math")
            self.assertEqual(package["homepage"], "https://example.com")
            self.assertEqual(package["wisp"], {"verification": url})
            self.assertEqual(npm["package/index.js"], node["package/index.js"])

    def test_undeclared_registries_get_no_archive(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            write_tar({"package/package.json": json_bytes({"name": "@demo/math", "version": "0.1.0"})}, directory / "node.tgz")
            publication_archives({}, "0.1.0", {"packages": {"node": {"name": "@demo/math"}}}, directory, "https://wisp.codes/x")
            self.assertFalse((directory / "npm.tgz").exists())
            self.assertFalse((directory / "cargo.crate").exists())


class PythonWheelTests(unittest.TestCase):
    def contract(self):
        return {"schema": 1, "package": "demo:math", "world": "math", "imports": [], "exports": [{"package": "demo:math", "name": "math-world", "types": [], "functions": [{"name": "add", "itemName": "add", "kind": "freestanding", "async": "sync", "params": [["left", "u32"], ["right", "u32"]], "result": "u32"}]}]}

    def test_wheel_uses_the_owner_name_and_links_its_verification_page(self):
        release = {"contract": self.contract(), "metadata": {"description": "Arithmetic"}}
        with tempfile.TemporaryDirectory() as temporary:
            wheel = make_wheel("Octocat.Math", "0.1.0", release, b"\0asm", Path(temporary), None, "https://wisp.codes/packages/octocat/math/math/0.1.0")
            self.assertEqual(wheel.name, "octocat_math-0.1.0-py3-none-any.whl")
            with zipfile.ZipFile(wheel) as archive:
                metadata = archive.read("octocat_math-0.1.0.dist-info/METADATA").decode()
            self.assertIn("Name: Octocat.Math\n", metadata)
            self.assertIn("Project-URL: Verification, https://wisp.codes/packages/octocat/math/math/0.1.0\n", metadata)
            with self.assertRaisesRegex(ValueError, "import name"):
                make_wheel("3d-math", "0.1.0", release, b"\0asm", Path(temporary))

    def test_unsupported_profiles_fail_before_wheel_publication(self):
        for modification in ["imports", "async", "resources"]:
            contract = self.contract()
            if modification == "imports":
                contract["imports"] = [{"package": "wasi:io"}]
            elif modification == "async":
                contract["exports"][0]["functions"][0]["async"] = "async"
            else:
                contract["exports"][0]["types"] = [{"name": "handle", "type": "resource"}]
            with self.subTest(profile=modification), self.assertRaises(ValueError):
                generate_source(contract)

    @unittest.skipUnless(importlib.util.find_spec("wasmtime"), "optional real Wasmtime 48 integration")
    def test_installed_wheel_executes_real_component(self):
        from wasmtime import wat2wasm
        component = bytes(wat2wasm('''(component
          (core module $m (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))
          (core instance $i (instantiate $m))
          (func (export "add") (param "left" u32) (param "right" u32) (result u32)
            (canon lift (core func $i "add"))))'''))
        release = {"contract": self.contract(), "metadata": {"description": "Real arithmetic"}}
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            wheel = make_wheel("octocat-arithmetic", "0.1.0", release, component, directory)
            with zipfile.ZipFile(wheel) as archive:
                metadata = archive.read("octocat_arithmetic-0.1.0.dist-info/METADATA")
                self.assertIn(b"Requires-Dist: wasmtime==48.0.0", metadata)
                archive.extractall(directory / "installed")
            spec = importlib.util.spec_from_file_location("octocat_arithmetic", directory / "installed/octocat_arithmetic/__init__.py", submodule_search_locations=[str(directory / "installed/octocat_arithmetic")])
            module = importlib.util.module_from_spec(spec)
            sys.modules["octocat_arithmetic"] = module
            try:
                spec.loader.exec_module(module)
                self.assertEqual(module.Client().add(20, 22), 42)
            finally:
                del sys.modules["octocat_arithmetic"]


if __name__ == "__main__":
    unittest.main()
