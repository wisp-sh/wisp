#!/usr/bin/env python3
"""Check that shared verifier snapshots never alias a mutable build binary."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from snapshot import WORKER_NAME, WORKER_VERSION, digest, freeze_cli


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        environment = patch.dict(os.environ, {"PATH": os.defpath}, clear=True)
        environment.start()
        self.addCleanup(environment.stop)
        build = self.root / "mutable-build"
        build.mkdir()
        self.build = build / "wisp"
        self.build.write_bytes(b"compiler CLI")
        worker = build / WORKER_NAME
        worker.write_text(f"#!/bin/sh\nprintf '%s\\n' '{WORKER_VERSION}'\n")
        worker.chmod(0o755)

    def test_mutating_build_cannot_change_frozen_parent(self):
        parent = self.root / "wisp"
        expected = freeze_cli(self.build, parent, allow_link=False)
        self.assertNotEqual(self.build.stat().st_ino, parent.stat().st_ino)
        self.build.write_bytes(b"new build")
        self.assertEqual(digest(parent), expected)

    def test_nested_verifier_links_only_the_identified_frozen_parent(self):
        parent = self.root / "wisp"
        expected = freeze_cli(self.build, parent, allow_link=False)
        (self.root / "frozen-cli.json").write_text(json.dumps({"schema": 1, "kind": "source-build-cli-snapshot", "sha256": expected}))
        environment = {"WISP_FROZEN_CLI": str(parent), "WISP_FROZEN_CLI_SHA256": expected}
        with patch.dict(os.environ, environment):
            nested = self.root / "nested-wisp"
            freeze_cli(parent, nested)
            self.assertEqual(parent.stat().st_ino, nested.stat().st_ino)
            self.assertEqual(digest(nested), expected)
            arbitrary = self.root / "arbitrary-wisp"
            freeze_cli(self.build, arbitrary)
            self.assertNotEqual(self.build.stat().st_ino, arbitrary.stat().st_ino)

    def test_changed_frozen_parent_is_rejected(self):
        parent = self.root / "wisp"
        expected = freeze_cli(self.build, parent, allow_link=False)
        (self.root / "frozen-cli.json").write_text(json.dumps({"schema": 1, "kind": "source-build-cli-snapshot", "sha256": expected}))
        parent.write_bytes(b"changed snapshot")
        with patch.dict(os.environ, {"WISP_FROZEN_CLI": str(parent), "WISP_FROZEN_CLI_SHA256": expected}):
            with self.assertRaisesRegex(ValueError, "parent CLI snapshot changed"):
                freeze_cli(parent, self.root / "nested-wisp")


if __name__ == "__main__":
    unittest.main()
