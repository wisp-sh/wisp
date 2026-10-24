"""Exercise paired snapshots with tiny executable workers, without Cargo."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from snapshot import WORKER_NAME, WORKER_VERSION, digest, freeze_cli, resolve_worker, verify_cli, worker_digest


class WorkerSnapshotTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.build = self.root / "build"
        self.build.mkdir()
        self.cli = self.build / "wisp"
        self.cli.write_bytes(b"CLI bytes")
        self.worker = self.make_worker(self.build / WORKER_NAME)
        self.output = self.root / "output"
        self.output.mkdir()
        self.frozen = self.output / "wisp"
        self.env = {"PATH": ""}

    def make_worker(self, path, version=WORKER_VERSION, suffix=""):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"#!/bin/sh\nprintf '%s\\n' '{version}'\n{suffix}")
        path.chmod(0o755)
        return path

    def freeze(self):
        cli_hash = freeze_cli(self.cli, self.frozen, environment=self.env)
        return cli_hash, worker_digest(self.frozen)

    def replay(self, hashes, environment=None):
        verify_cli(self.frozen, *hashes, environment=self.env if environment is None else environment)

    def mark_parent(self, cli_hash):
        (self.output / "frozen-cli.json").write_text(json.dumps({
            "schema": 1, "kind": "source-build-cli-snapshot", "sha256": cli_hash,
        }))
        self.env.update(WISP_FROZEN_CLI=str(self.frozen), WISP_FROZEN_CLI_SHA256=cli_hash)

    def test_freezes_both_binaries_and_pins_worker_environment(self):
        hashes = self.freeze()
        frozen_worker = self.output / WORKER_NAME
        self.assertEqual(self.env["WISP_WORKER"], str(frozen_worker))
        self.assertNotEqual(self.cli.stat().st_ino, self.frozen.stat().st_ino)
        self.assertNotEqual(self.worker.stat().st_ino, frozen_worker.stat().st_ino)
        receipt = json.loads((self.output / "frozen-worker.json").read_text())
        self.assertEqual(receipt["sha256"], hashes[1])
        self.cli.write_bytes(b"rebuilt CLI")
        self.worker.write_bytes(b"rebuilt worker")
        self.replay(hashes)

    def test_explicit_worker_takes_precedence_over_sibling_and_path(self):
        explicit = self.make_worker(self.root / "explicit", suffix="# selected worker\n")
        self.env["WISP_WORKER"] = str(explicit)
        self.freeze()
        self.assertEqual(worker_digest(self.frozen), digest(explicit))
        self.assertNotEqual(worker_digest(self.frozen), digest(self.worker))

    def test_missing_explicit_worker_does_not_fall_back(self):
        self.env["WISP_WORKER"] = str(self.root / "missing")
        with self.assertRaisesRegex(ValueError, "WISP_WORKER"):
            self.freeze()
        self.assertFalse(self.frozen.exists())

    def test_empty_explicit_worker_does_not_fall_back(self):
        self.env["WISP_WORKER"] = ""
        with self.assertRaisesRegex(ValueError, "WISP_WORKER"):
            self.freeze()

    def test_relative_explicit_worker(self):
        self.env["WISP_WORKER"] = str(self.worker.relative_to(self.root))
        previous = Path.cwd()
        try:
            os.chdir(self.root)
            self.freeze()
        finally:
            os.chdir(previous)
        self.assertEqual(worker_digest(self.frozen), digest(self.worker))

    def test_path_fallback(self):
        path_worker = self.root / "path" / WORKER_NAME
        path_worker.parent.mkdir()
        self.worker.rename(path_worker)
        self.env["PATH"] = str(path_worker.parent)
        self.assertEqual(resolve_worker(self.cli, self.env), path_worker)
        self.freeze()

    def test_missing_worker_has_actionable_error(self):
        self.worker.unlink()
        with self.assertRaisesRegex(ValueError, "missing wisp-worker.*WISP_WORKER"):
            self.freeze()

    def test_incompatible_worker_is_rejected_before_copy(self):
        self.make_worker(self.worker, "wisp-worker 0.2.0 wasmtime 48.0.0 protocol 1")
        with self.assertRaisesRegex(ValueError, "incompatible wisp-worker"):
            self.freeze()
        self.assertFalse(self.frozen.exists())

    def test_nonexecutable_worker_has_clear_error(self):
        self.worker.chmod(0o644)
        with self.assertRaisesRegex(ValueError, "cannot execute wisp-worker"):
            self.freeze()

    def test_replay_detects_modified_cli(self):
        hashes = self.freeze()
        self.frozen.write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "packaging executable missing or changed"):
            self.replay(hashes)

    def test_replay_detects_modified_worker(self):
        hashes = self.freeze()
        (self.output / WORKER_NAME).write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "frozen wisp-worker missing or changed"):
            self.replay(hashes)

    def test_replay_missing_worker_does_not_use_path(self):
        hashes = self.freeze()
        (self.output / WORKER_NAME).unlink()
        with self.assertRaisesRegex(ValueError, "frozen wisp-worker missing or changed"):
            self.replay(hashes, {"PATH": str(self.build)})

    def test_replay_without_worker_hash_requires_new_snapshot(self):
        hashes = self.freeze()
        with self.assertRaisesRegex(ValueError, "missing worker hash"):
            self.replay((hashes[0], None))

    def test_replay_rejects_a_different_explicit_worker(self):
        hashes = self.freeze()
        explicit = self.make_worker(self.root / "other-worker", suffix="# different bytes\n")
        with self.assertRaisesRegex(ValueError, "WISP_WORKER differs"):
            self.replay(hashes, {"WISP_WORKER": str(explicit)})

    def test_replay_respects_identical_explicit_worker(self):
        hashes = self.freeze()
        env = {"WISP_WORKER": str(self.worker)}
        self.replay(hashes, env)
        self.assertEqual(env["WISP_WORKER"], str(self.worker))

    def test_replay_missing_explicit_worker_does_not_fall_back(self):
        hashes = self.freeze()
        with self.assertRaisesRegex(ValueError, "WISP_WORKER"):
            self.replay(hashes, {"WISP_WORKER": str(self.root / "missing")})

    def test_nested_snapshot_links_cli_and_copies_worker(self):
        cli_hash, worker_hash = self.freeze()
        self.mark_parent(cli_hash)
        nested = self.root / "nested"
        nested.mkdir()
        freeze_cli(self.frozen, nested / "wisp", environment=self.env)
        self.assertEqual(self.frozen.stat().st_ino, (nested / "wisp").stat().st_ino)
        self.assertNotEqual((self.output / WORKER_NAME).stat().st_ino, (nested / WORKER_NAME).stat().st_ino)
        self.assertEqual(worker_digest(nested / "wisp"), worker_hash)

    def test_nested_snapshot_rejects_changed_parent_cli(self):
        cli_hash, _ = self.freeze()
        self.mark_parent(cli_hash)
        self.frozen.write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "parent CLI snapshot changed"):
            freeze_cli(self.frozen, self.output / "nested-wisp", environment=self.env)

    def test_nested_snapshot_rejects_changed_parent_worker_without_execution(self):
        cli_hash, _ = self.freeze()
        self.mark_parent(cli_hash)
        (self.output / WORKER_NAME).write_bytes(b"tampered")
        with patch("snapshot.check_worker") as check:
            with self.assertRaisesRegex(ValueError, "parent worker snapshot changed"):
                freeze_cli(self.frozen, self.output / "nested-wisp", environment=self.env)
            check.assert_not_called()

    def test_unidentified_cli_is_copied(self):
        cli_hash, _ = self.freeze()
        self.mark_parent(cli_hash)
        arbitrary = self.root / "arbitrary"
        arbitrary.mkdir()
        freeze_cli(self.cli, arbitrary / "wisp", environment=self.env)
        self.assertNotEqual(self.cli.stat().st_ino, (arbitrary / "wisp").stat().st_ino)


if __name__ == "__main__":
    unittest.main()
