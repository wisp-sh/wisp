#!/usr/bin/env python3
"""Negative controls for the compiler acquisition boundary."""
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import acquire


class AcquisitionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.value = {"schema": 1, "platform": "linux-x86_64", "bundles": [
            {"tool": tool, "version": "0.1.0", "url": f"https://example.org/{tool}.wisp.zip",
             "size": 8, "sha256": "a" * 64} for tool in sorted(acquire.TOOLS)]}

    def read(self, value=None):
        path = self.root / "manifest.json"
        path.write_text(json.dumps(self.value if value is None else value))
        return acquire.manifest(path, acquire.digest(path))

    def test_all_five_bundles_are_required(self):
        self.assertEqual(self.read(), self.value)
        self.value["bundles"].pop()
        with self.assertRaisesRegex(ValueError, "five"):
            self.read()

    def test_duplicate_and_unknown_tools_are_rejected(self):
        self.value["bundles"][0]["tool"] = self.value["bundles"][1]["tool"]
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.read()
        self.value["bundles"][0]["tool"] = "native-compiler"
        with self.assertRaisesRegex(ValueError, "unknown"):
            self.read()

    def test_invalid_bundle_bounds_and_hashes_are_rejected(self):
        for key, invalid in (("size", 0), ("size", acquire.MAX_BUNDLE + 1), ("size", True),
                             ("sha256", "A" * 64), ("version", "../0.1.0")):
            value = copy.deepcopy(self.value)
            value["bundles"][0][key] = invalid
            with self.subTest(key=key, invalid=invalid), self.assertRaises(ValueError):
                self.read(value)

    def test_unknown_fields_platform_and_schema_are_rejected(self):
        for key, invalid in (("extra", True), ("platform", "Darwin-arm64"), ("schema", True)):
            value = copy.deepcopy(self.value)
            value[key] = invalid
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.read(value)

    def test_urls_must_be_public_https_without_secrets(self):
        for url in ("http://example.org/file", "file:///tmp/compiler", "https://user:password@example.org/file",
                    "https://example.org/file?token=secret", "https://example.org/file#fragment"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                acquire.public_https(url)

    def test_modified_manifest_and_duplicate_json_keys_are_rejected(self):
        path = self.root / "manifest.json"
        path.write_text(json.dumps(self.value))
        expected = acquire.digest(path)
        path.write_text(path.read_text() + " ")
        with self.assertRaisesRegex(ValueError, "checksum"):
            acquire.manifest(path, expected)
        with self.assertRaisesRegex(ValueError, "duplicate JSON"):
            acquire.strict_json('{"schema":1,"schema":2}')

    def test_cached_file_hash_size_and_symlink_are_verified(self):
        path = self.root / "bundle"
        path.write_bytes(b"compiler")
        expected = acquire.digest(path)
        self.assertEqual(acquire.verify_file(path, expected, 8), path)
        for digest, size in (("0" * 64, 8), (expected, 9)):
            with self.assertRaises(ValueError):
                acquire.verify_file(path, digest, size)
        link = self.root / "link"
        link.symlink_to(path)
        with self.assertRaisesRegex(ValueError, "regular"):
            acquire.verify_file(link, expected, 8)

    def test_download_hash_and_length_are_checked_before_admission(self):
        payload = b"compiler"
        expected = hashlib.sha256(payload).hexdigest()

        class Response(io.BytesIO):
            url = "https://cdn.example.org/bundle"
            headers = {"Content-Length": "8"}

        path = self.root / "bundle"
        with patch("urllib.request.urlopen", return_value=Response(payload)):
            self.assertEqual(acquire.download("https://example.org/bundle", expected, path, size=8), path)
        path.unlink()
        with patch("urllib.request.urlopen", return_value=Response(b"tampered")), self.assertRaisesRegex(ValueError, "checksum"):
            acquire.download("https://example.org/bundle", expected, path, size=8)
        self.assertFalse(path.exists())
        self.assertFalse(path.with_name("bundle.partial").exists())
        with patch("urllib.request.urlopen", return_value=Response(payload)), self.assertRaisesRegex(ValueError, "Content-Length"):
            acquire.download("https://example.org/bundle", expected, path, size=7)

    def test_downgraded_redirect_is_rejected(self):
        class Response(io.BytesIO):
            url = "http://cdn.example.org/bundle"
            headers = {}
        with patch("urllib.request.urlopen", return_value=Response(b"compiler")), self.assertRaisesRegex(ValueError, "HTTPS"):
            acquire.download("https://example.org/bundle", "a" * 64, self.root / "bundle")


if __name__ == "__main__":
    unittest.main()
