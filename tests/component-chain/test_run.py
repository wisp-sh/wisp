#!/usr/bin/env python3
"""Exercise real composition failures after run.py builds the positive chain."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]

# This Component validates on its own. Its exported interface uses the expected
# record and error names, but total returns u32 instead of the required result.
INCOMPATIBLE_LIBRARY = """(component
  (type $line-item (record (field "quantity" u32) (field "unit-price" u64)))
  (type $calculation-error (enum "overflow"))
  (core module $m
    (func (export "total") (param i32 i64) (result i32) i32.const 1))
  (core instance $m (instantiate $m))
  (func $total (param "item" $line-item) (result u32)
    (canon lift (core func $m "total")))
  (instance $arithmetic
    (export "line-item" (type $line-item))
    (export "calculation-error" (type $calculation-error))
    (export "total" (func $total)))
  (export "wisp:library-chain/arithmetic@0.1.0" (instance $arithmetic)))
"""


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class CompositionControls(unittest.TestCase):
    tools = None
    output = None
    commands = []

    @classmethod
    def setUpClass(cls):
        cls.library = cls.output / "library.wasm"
        cls.consumer = cls.output / "consumer.wasm"
        cls.composed = cls.output / "composed.wasm"
        for path in (cls.library, cls.consumer, cls.composed):
            if not path.is_file():
                raise RuntimeError(f"run run.py first; missing artifact {path}")
        cls.original_hashes = {
            str(path): digest(path) for path in (cls.library, cls.consumer, cls.composed)
        }

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="negative-", dir=self.output)
        self.work = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)

    def tearDown(self):
        for name, expected in self.original_hashes.items():
            self.assertEqual(digest(Path(name)), expected, f"input changed: {name}")

    def command(self, *args):
        result = subprocess.run(
            [str(value) for value in args],
            cwd=self.work,
            env={**os.environ, "CARGO_NET_OFFLINE": "true"},
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.commands.append({
            "test": self.id(),
            "argv": [str(value) for value in args],
            "returncode": result.returncode,
            "stdout": result.stdout,
            "stderr": result.stderr,
        })
        return result

    def compose(self, library, output):
        return self.command(
            self.tools / "wac", "compose", HERE / "chain.wac",
            "--dep", f"wisp:library={library}",
            "--dep", f"wisp:consumer={self.consumer}",
            "--output", output,
        )

    def fixture(self, text):
        wat = self.work / "library.wat"
        wasm = self.work / "library.wasm"
        wat.write_text(text)
        parsed = self.command(self.tools / "wasm-tools", "parse", wat, "-o", wasm)
        self.assertEqual(parsed.returncode, 0, parsed.stderr)
        valid = self.command(self.tools / "wasm-tools", "validate", wasm)
        self.assertEqual(valid.returncode, 0, valid.stderr)
        return wasm

    def test_missing_dependency_fails_before_output(self):
        output = self.work / "composed.wasm"
        result = self.compose(self.work / "missing.wasm", output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("wisp:library", result.stderr)
        self.assertIn("does not exist", result.stderr)
        self.assertFalse(output.exists())

    def test_wrong_interface_fails_before_output(self):
        library = self.fixture(INCOMPATIBLE_LIBRARY.replace(
            '"wisp:library-chain/arithmetic@0.1.0"',
            '"wisp:library-chain/another@0.1.0"',
        ))
        output = self.work / "composed.wasm"
        result = self.compose(library, output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("wisp:library-chain/arithmetic@0.1.0", result.stderr)
        self.assertIn("export", result.stderr)
        self.assertFalse(output.exists())

    def test_incompatible_signature_fails_before_output(self):
        library = self.fixture(INCOMPATIBLE_LIBRARY)
        output = self.work / "composed.wasm"
        result = self.compose(library, output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mismatched type for function result", result.stderr)
        self.assertIn("expected result, found u32", result.stderr)
        self.assertFalse(output.exists())

    def test_failed_composition_preserves_existing_output(self):
        library = self.fixture(INCOMPATIBLE_LIBRARY)
        output = self.work / "composed.wasm"
        previous = self.composed.read_bytes()
        output.write_bytes(previous)
        result = self.compose(library, output)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(output.read_bytes(), previous)

    def test_invalid_component_fails_before_output(self):
        library = self.work / "invalid.wasm"
        library.write_bytes(b"\x00asm\x0d\x00\x01\x00\xff")
        output = self.work / "composed.wasm"
        result = self.compose(library, output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("wisp:library", result.stderr)
        self.assertFalse(output.exists())

    def test_uncomposed_consumer_requires_its_component_dependency(self):
        result = self.command(
            self.tools / "wasmtime", "run", "--invoke",
            "total({quantity: 3, unit-price: 7})", self.consumer,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("wisp:library-chain/arithmetic@0.1.0", result.stderr)
        self.assertIn("implementation", result.stderr)

    def test_evidence_binds_the_executed_components(self):
        report = json.loads((self.output / "report.json").read_text())
        self.assertEqual(report["target"], "wasm32-unknown-unknown")
        for name in ("library", "consumer", "composed", "composed-repeat"):
            path = self.output / f"{name}.wasm"
            self.assertEqual(report["artifacts"][name]["sha256"], digest(path))
        self.assertEqual(report["host_imports"], {
            "library": [],
            "consumer": ["wisp:library-chain/arithmetic@0.1.0"],
            "composed": [],
        })
        for name, expected in {
            "success": "ok(21)", "overflow": "err(overflow)", "zero": "ok(0)",
        }.items():
            self.assertEqual(report["cases"][name]["library"], expected)
            self.assertEqual(report["cases"][name]["composed"], expected)

    def test_failed_proof_removes_stale_success_evidence(self):
        output = self.work / "failed-proof"
        output.mkdir()
        report = output / "report.json"
        report.write_text('{"cases":{"success":{"composed":"ok(21)"}}}\n')
        result = self.command(
            sys.executable, HERE / "run.py", "--tools-dir", self.work / "absent-tools",
            "--output-dir", output,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(report.exists())

    def test_modified_tool_is_rejected_before_execution(self):
        tools = self.work / "tools"
        tools.mkdir()
        (tools / "installed.json").write_bytes((self.tools / "installed.json").read_bytes())
        marker = self.work / "unexpected-execution"
        binary = tools / "wasm-tools"
        binary.write_text(f'#!/bin/sh\ntouch "{marker}"\n')
        binary.chmod(0o755)
        result = self.command(
            sys.executable, HERE / "run.py", "--tools-dir", tools,
            "--output-dir", self.work / "failed-proof",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("modified tool: wasm-tools", result.stderr)
        self.assertFalse(marker.exists())


class RecordedResult(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.rows = []

    def addSuccess(self, test):
        self.rows.append({"test": test.id(), "status": "pass"})
        super().addSuccess(test)

    def addFailure(self, test, error):
        self.rows.append({"test": test.id(), "status": "fail"})
        super().addFailure(test, error)

    def addError(self, test, error):
        self.rows.append({"test": test.id(), "status": "error"})
        super().addError(test, error)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools-dir", type=Path, default=ROOT / ".tools/component-library-chain")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "out/component-library-chain")
    parser.add_argument("--evidence", type=Path, default=ROOT / "evidence/component-library-chain/negative-controls.json")
    args = parser.parse_args()
    CompositionControls.tools = args.tools_dir.resolve()
    CompositionControls.output = args.output_dir.resolve()
    CompositionControls.commands = []
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    args.evidence.unlink(missing_ok=True)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(CompositionControls)
    result = unittest.TextTestRunner(verbosity=2, resultclass=RecordedResult).run(suite)
    args.evidence.write_text(json.dumps({
        "success": result.wasSuccessful(),
        "testsRun": result.testsRun,
        "rows": result.rows,
        "commands": CompositionControls.commands,
        "inputHashes": getattr(CompositionControls, "original_hashes", {}),
        "testSourceSha256": digest(Path(__file__)),
        "scope": "Real WAC composition rejection controls; no package-generation support is claimed.",
    }, indent=2) + "\n")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
