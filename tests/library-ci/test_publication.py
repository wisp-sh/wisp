"""Publication regressions using real wheels and mocked public registry JSON."""

import copy
import io
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError, URLError

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts/ci"))
from build import manifest
from common import json_bytes, sha256
from publish import collect_wheels, plan, prepare_pypi
from python_wheel import make_wheel, wheel_filename
from record import load_artifacts

CONTRACT = {"schema": 1, "package": "demo:math", "world": "math", "imports": [], "exports": [
    {"package": "demo:math", "name": "math-world", "types": [], "functions": [
        {"name": "answer", "itemName": "answer", "kind": "freestanding", "async": "sync", "params": [], "result": "u32"}]}]}


def artifacts(root, specs):
    root.mkdir()
    packages = []
    for name, target, version in specs:
        folder = root / "packages" / name
        folder.mkdir(parents=True)
        release = {"contract": CONTRACT, "version": version}
        (folder / "component.wasm").write_bytes(b"\0asm")
        (folder / "library-release.json").write_bytes(json_bytes(release))
        make_wheel(target, version, release, b"\0asm", folder)
        files = [{"name": path.name, "size": path.stat().st_size, "sha256": sha256(path.read_bytes()), "mediaType": "application/octet-stream"}
                 for path in sorted(folder.iterdir())]
        packages.append({"name": name, "path": name, "version": version, "targets": {"pypi": target}, "files": files})
    value = {"schema": 2, "repository": "octocat/math", "packages": packages}
    (root / "manifest.json").write_bytes(json_bytes(value))
    return value


def release_json(package):
    wheel = next(file for file in package["files"] if file["name"].endswith(".whl"))
    return {"info": {"name": package["targets"]["pypi"], "version": package["version"]}, "last_serial": 314159,
            "urls": [{"filename": wheel["name"], "packagetype": "bdist_wheel", "python_version": "py3",
                      "digests": {"sha256": wheel["sha256"], "md5": "a" * 32}, "size": wheel["size"],
                      "url": "https://files.pythonhosted.org/packages/00/00/" + wheel["name"],
                      "yanked": False, "yanked_reason": None}], "vulnerabilities": []}


class RegistryIdentityTests(unittest.TestCase):
    def test_manifest_rejects_registry_aliases_before_building(self):
        cases = [("pypi", "Foo.Bar", "foo-bar"), ("pypi", "FOO._--bar", "foo_bar"),
                 ("cargo", "Foo_Bar", "foo-bar"), ("npm", "@owner/foo", "@owner/foo")]
        for registry, first, second in cases:
            with self.subTest(registry=registry, first=first), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "first").mkdir()
                (root / "second").mkdir()
                packages = [{"name": name, "path": name, "targets": {registry: target}}
                            for name, target in [("first", first), ("second", second)]]
                (root / ".wisp.json").write_bytes(json_bytes({"schema": 2, "packages": packages}))
                with self.assertRaisesRegex(ValueError, f"duplicate {registry} registry identity"):
                    manifest(root)

    def test_artifact_admission_and_plan_reject_aliases_even_at_different_versions(self):
        for version in ["1.0.0", "2.0.0"]:
            with self.subTest(version=version), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                artifacts(root / "artifacts", [("first", "Foo.Bar", "1.0.0"), ("second", "foo-bar", version)])
                with self.assertRaisesRegex(ValueError, "duplicate pypi registry identity"):
                    load_artifacts(root / "artifacts")
                with self.assertRaisesRegex(ValueError, "duplicate pypi registry identity"):
                    plan(root / "artifacts", root / "wheels", root / "output")
                self.assertFalse((root / "wheels").exists())
                self.assertFalse((root / "output").exists())

    def test_wheel_must_match_declared_name_and_stable_version(self):
        for target, version in [("other", "1.0.0"), ("foo-bar", "2.0.0"), ("foo-bar", "1.0.0-rc.1")]:
            with self.subTest(target=target, version=version), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "artifacts"
                value = artifacts(root, [("first", "foo-bar", "1.0.0")])
                value["packages"][0].update(targets={"pypi": target}, version=version)
                (root / "manifest.json").write_bytes(json_bytes(value))
                with self.assertRaises(ValueError):
                    load_artifacts(root)

    def test_plan_does_not_overwrite_even_if_admission_misses_a_collision(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = artifacts(root / "artifacts", [("first", "foo-bar", "1.0.0")])
            _, files = load_artifacts(root / "artifacts")
            key = ("first", wheel_filename("foo-bar", "1.0.0"))
            files[("second", key[1])] = files[key]
            with patch("publish.load_artifacts", return_value=(value, files)), self.assertRaisesRegex(ValueError, "duplicate wheel"):
                plan(root / "artifacts", root / "wheels", root / "output")
            self.assertFalse((root / "wheels").exists())

    def test_collector_rejects_existing_directory_and_changed_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifacts(root / "artifacts", [("first", "foo-bar", "1.0.0")])
            _, files = load_artifacts(root / "artifacts")
            entry = files[("first", wheel_filename("foo-bar", "1.0.0"))]
            collect_wheels([entry], root / "wheels")
            original = (root / "wheels" / entry[0].name).read_bytes()
            with self.assertRaises(FileExistsError):
                collect_wheels([entry], root / "wheels")
            self.assertEqual((root / "wheels" / entry[0].name).read_bytes(), original)
            entry[0].write_bytes(b"replaced after admission")
            with self.assertRaisesRegex(ValueError, "changed after verification"):
                collect_wheels([entry], root / "changed")

    def test_wheel_writer_itself_never_replaces_a_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            wheel = make_wheel("Foo.Bar", "1.0.0", {"contract": CONTRACT}, b"first", root)
            original = wheel.read_bytes()
            with self.assertRaises(FileExistsError):
                make_wheel("foo-bar", "1.0.0", {"contract": CONTRACT}, b"second", root)
            self.assertEqual(wheel.read_bytes(), original)

    def test_documented_schema_2_example_passes_manifest_validation(self):
        documentation = Path(__file__).resolve().parents[2] / "docs/releases/library-workflow.md"
        example = json.loads(re.search(r"```json\n(.*?)\n```", documentation.read_text(), re.S)[1])
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for package in example["packages"]:
                (root / package["path"]).mkdir(parents=True)
            (root / ".wisp.json").write_bytes(json_bytes(example))
            self.assertEqual(manifest(root), example)


class PyPIPreparationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.artifacts = self.root / "artifacts"
        self.value = artifacts(self.artifacts, [("first", "Foo.Bar", "1.0.0")])
        self.release = release_json(self.value["packages"][0])
        self.wheels = self.root / "wheels"
        self.output = self.root / "output"
        self.url = "https://pypi.org/pypi/foo-bar/1.0.0/json"
        self.network = self.enterContext(patch("publish.HTTP.open"))

    def prepare(self, response):
        self.network.return_value = io.BytesIO(json_bytes(response))
        return prepare_pypi(self.artifacts, self.wheels, self.output)

    def test_only_identical_file_is_skipped_with_a_normalized_project_lookup(self):
        # PyPI treats 1.0 and 1.0.0 as the same release. It can retain the
        # metadata spelling from the first upload; select the actual filename.
        self.release["info"]["version"] = "1.0"
        self.assertFalse(self.prepare(self.release))
        self.assertEqual(self.network.call_args.args[0].full_url, self.url)
        self.assertEqual(list(self.wheels.iterdir()), [])
        self.assertEqual(self.output.read_text(), "pypi=false\n")

    def test_404_stages_the_verified_wheel(self):
        self.network.side_effect = HTTPError(self.url, 404, "Not Found", {}, None)
        self.assertTrue(prepare_pypi(self.artifacts, self.wheels, self.output))
        filename = self.release["urls"][0]["filename"]
        self.assertEqual((self.wheels / filename).read_bytes(), (self.artifacts / "packages/first" / filename).read_bytes())
        self.assertEqual(self.output.read_text(), "pypi=true\n")

    def test_existing_release_with_other_files_still_needs_our_exact_filename(self):
        cases = [[], [{**self.release["urls"][0], "filename": "foo_bar-1.0.0.tar.gz", "packagetype": "sdist"}],
                 [{**self.release["urls"][0], "filename": "foo_bar-1.0.0-cp311-cp311-linux_x86_64.whl"}],
                 [{**self.release["urls"][0], "filename": "foo_bar-1.0-py3-none-any.whl"}],
                 [{**self.release["urls"][0], "filename": "foo_bar-2.0.0-py3-none-any.whl"}]]
        for index, entries in enumerate(cases):
            with self.subTest(entries=entries):
                self.wheels = self.root / f"wheels-{index}"
                self.output = self.root / f"output-{index}"
                self.assertTrue(self.prepare({**self.release, "urls": entries}))
                self.assertEqual([p.name for p in self.wheels.iterdir()], [self.release["urls"][0]["filename"]])

    def test_mismatch_missing_integrity_and_yanked_files_fail_without_success_outputs(self):
        changes = [{"digests": {"sha256": "0" * 64}}, {"digests": {}}, {"digests": None},
                   {"size": 1}, {"size": "123"}, {"packagetype": "sdist"}, {"yanked": True}, {"yanked": None}]
        for change in changes:
            with self.subTest(change=change):
                response = copy.deepcopy(self.release)
                response["urls"][0].update(change)
                with self.assertRaises(ValueError):
                    self.prepare(response)
                self.assertFalse(self.wheels.exists())
                self.assertFalse(self.output.exists())

    def test_malformed_registry_responses_are_not_missing_releases(self):
        for response in [None, [], {}, {"urls": None}, {"urls": [{}]},
                         {"urls": [self.release["urls"][0]] * 2}]:
            with self.subTest(response=response), self.assertRaises(ValueError):
                self.prepare(response)
        self.network.return_value = io.BytesIO(b'{"urls": [], "urls": []}')
        with self.assertRaisesRegex(ValueError, "duplicate JSON"):
            prepare_pypi(self.artifacts, self.wheels, self.output)
        self.assertFalse(self.output.exists())

    def test_registry_failures_remain_failures(self):
        for error in [HTTPError(self.url, code, "Failure", {}, None) for code in [403, 429, 500]] + [URLError("timeout")]:
            with self.subTest(error=error):
                self.network.side_effect = error
                with self.assertRaises(type(error)):
                    prepare_pypi(self.artifacts, self.wheels, self.output)
                self.assertFalse(self.wheels.exists())
                self.assertFalse(self.output.exists())

    def test_artifacts_are_verified_before_registry_requests(self):
        filename = self.release["urls"][0]["filename"]
        (self.artifacts / "packages/first" / filename).write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            prepare_pypi(self.artifacts, self.wheels, self.output)
        self.network.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_mixed_projects_stage_only_missing_files_and_check_all_before_staging(self):
        second_root = self.root / "mixed"
        value = artifacts(second_root, [("first", "foo-bar", "1.0.0"), ("second", "another-project", "2.0.0")])
        first = release_json(value["packages"][0])
        second = release_json(value["packages"][1])
        responses = [first, {**second, "urls": []}]
        self.network.side_effect = lambda request, **kwargs: io.BytesIO(json_bytes(responses.pop(0)))
        self.assertTrue(prepare_pypi(second_root, self.wheels, self.output))
        self.assertEqual([p.name for p in self.wheels.iterdir()], [second["urls"][0]["filename"]])
        second["urls"][0]["digests"]["sha256"] = "0" * 64
        responses[:] = [{**first, "urls": []}, second]
        with self.assertRaisesRegex(ValueError, "different bytes"):
            prepare_pypi(second_root, self.root / "failed-wheels", self.root / "failed-output")
        self.assertFalse((self.root / "failed-wheels").exists())
        self.assertFalse((self.root / "failed-output").exists())


if __name__ == "__main__":
    unittest.main()
