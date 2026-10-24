"""Replay runners must reject changed snapshot bytes before running consumers."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from snapshot import WORKER_NAME, digest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/library-chain"))


class LibraryWorkerReplayTests(unittest.TestCase):
    def test_replay_rejects_missing_worker_and_legacy_or_changed_hashes(self):
        for name in ["chain", "async", "error-context", "network"]:
            spec = importlib.util.spec_from_file_location(
                f"library_{name.replace('-', '_')}_runner", ROOT / f"tests/library-{name}/run.py")
            runner = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(runner)
            for defect in ["missing worker", "changed worker", "changed CLI", "legacy receipt"]:
                with self.subTest(runner=name, defect=defect), tempfile.TemporaryDirectory() as temporary:
                    output = Path(temporary)
                    cli = output / ("packaging-wisp" if name in ["chain", "network"] else "wisp")
                    cli.write_bytes(b"CLI snapshot")
                    worker = output / WORKER_NAME
                    worker.write_bytes(b"worker snapshot")
                    cli_key = "wisp" if name == "network" else "binary_sha256" if name == "chain" else "sha256"
                    receipt = {cli_key: digest(cli), "worker_sha256": digest(worker)}
                    if defect == "missing worker":
                        worker.unlink()
                    elif defect == "changed worker":
                        worker.write_bytes(b"changed worker")
                    elif defect == "changed CLI":
                        cli.write_bytes(b"changed CLI")
                    else:
                        receipt.pop("worker_sha256")
                    receipt_name = "inputs.json" if name == "network" else "packaging-tool.json"
                    (output / receipt_name).write_text(json.dumps(receipt))
                    (output / "release").mkdir()
                    (output / "release/library-release.json").write_text("{}")
                    argv = ["run.py", "--replay", "--output-dir", str(output)]
                    if name == "network":
                        argv.extend(["--build-dir", str(output / "target")])
                    message = ("packaging executable" if defect == "changed CLI" else
                               "missing worker hash" if defect == "legacy receipt" else "frozen wisp-worker")
                    with patch.object(sys, "argv", argv), patch.dict(os.environ, {"PATH": os.defpath}, clear=True), \
                            patch.object(runner.subprocess, "run", side_effect=AssertionError("consumer ran before hashes were checked")):
                        with self.assertRaisesRegex(ValueError, message):
                            runner.main()


if __name__ == "__main__":
    unittest.main()
