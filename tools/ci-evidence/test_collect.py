import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import collect


class CollectionTests(unittest.TestCase):
    def test_failed_build_tail_and_receipts_exclude_caches_and_secrets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "evidence/source-build"
            evidence.mkdir(parents=True)
            secret = "private-test-token"
            (evidence / "cli-build.log").write_text("x" * collect.LOG_BYTES + f"\nerror {root} {secret}\n")
            (evidence / "setup-inputs.json").write_text(json.dumps({"hash": "abc", "prefix": str(root),
                "url": "https://user:password@example.org/input?token=private", "accessToken": "hidden",
                "credentialForwardingRejected": True}))
            (evidence / "private.env").write_text(secret)
            (root / "target").mkdir()
            (root / "target/cache.json").write_text(secret)
            result = collect.collect(root, root / "upload", "source-build", {"TEST_TOKEN": secret})
            log = (root / "upload/evidence/source-build/cli-build.log").read_text()
            self.assertIn("error $WORKSPACE [REDACTED]", log)
            self.assertLessEqual(len(log.encode()), collect.LOG_BYTES)
            self.assertTrue(result["files"][0]["tailOnly"])
            receipt = json.loads((root / "upload/evidence/source-build/setup-inputs.json").read_text())
            self.assertEqual(receipt["hash"], "abc")
            self.assertEqual(receipt["accessToken"], "[REDACTED]")
            self.assertTrue(receipt["credentialForwardingRejected"])
            self.assertNotIn("password", receipt["url"])
            self.assertNotIn("private", receipt["url"])
            self.assertFalse((root / "upload/target").exists())
            self.assertFalse((root / "upload/evidence/source-build/private.env").exists())

    def test_oversized_json_and_symlinks_are_omitted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / "evidence/source-build"
            evidence.mkdir(parents=True)
            (evidence / "large.json").write_bytes(b" " * (collect.JSON_BYTES + 1))
            (root / "private").write_text("private")
            (evidence / "linked.log").symlink_to(root / "private")
            (evidence / "invalid.json").write_text("incomplete")
            result = collect.collect(root, root / "upload", "source-build", {})
            self.assertEqual(len(result["files"]), 3)
            self.assertTrue(all("omitted" in entry for entry in result["files"]))
            self.assertEqual(result["savedBytes"], 0)

    def test_count_and_total_limits_and_empty_failure_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result = collect.collect(root, root / "empty", "compose", {})
            self.assertEqual(result["files"], [])
            evidence = root / "evidence/component-library-chain"
            evidence.mkdir(parents=True)
            for index in range(4):
                (evidence / f"{index}.log").write_bytes(b"a" * 1024)
            with patch.object(collect, "MAX_FILES", 4), patch.object(collect, "TOTAL_BYTES", 64 * 1024 + 1500):
                result = collect.collect(root, root / "upload", "compose", {})
            self.assertTrue(result["fileLimitReached"])
            self.assertEqual(result["savedBytes"], 1024)
            self.assertEqual(len(result["files"]), 3)


if __name__ == "__main__":
    unittest.main()
