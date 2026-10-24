#!/usr/bin/env python3
"""Exercise the actual Bend checker/compiler Component, after bootstrap.py."""
import unittest

from compile import compile_request, integrity


class BendCompilerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        integrity()

    def compile(self, source, **files):
        files["main.bend"] = source
        return compile_request({"entry": "main.bend", "files": {
            "/project/" + name: text for name, text in files.items()
        }})

    def test_numeric_exports(self):
        result = self.compile("import Base\ndef add(a: U32, b: U32) -> U32:\n  (a + b : U32)\n"
                              "def multiply(a: F32, b: F32) -> F32:\n  (a * b : F32)\n")
        self.assertEqual(result["exports"], ["add", "multiply"])

    def test_local_import(self):
        result = self.compile("import Base\nimport helper.bend as Helper\n"
                              "def add(a: U32, b: U32) -> U32:\n  Helper.sum(a, b)\n",
                              **{"helper.bend": "import Base\ndef sum(a: U32, b: U32) -> U32:\n  (a + b : U32)\n"})
        self.assertEqual(result["exports"], ["add"])

    def test_type_error(self):
        with self.assertRaisesRegex(ValueError, "expected"):
            self.compile("import Base\ndef bad() -> U32:\n  1.5\n")

    def test_missing_proof(self):
        with self.assertRaisesRegex(ValueError, "unproved"):
            self.compile("import Base\nlaw missing:\n  {0 == 0 : U32}\n"
                         "def answer() -> U32:\n  42\n")

    def test_completed_proof(self):
        result = self.compile("import Base\nlaw reflexive:\n  {0 == 0 : U32}\n"
                              "def reflexive():\n  {==}\n"
                              "def answer() -> U32:\n  42\n")
        self.assertEqual(result["exports"], ["answer"])

    def test_false_proof(self):
        with self.assertRaises(ValueError):
            self.compile("import Base\nlaw wrong:\n  {0 == 1 : U32}\n"
                         "def wrong():\n  {==}\n"
                         "def answer() -> U32:\n  42\n")

    def test_missing_import(self):
        with self.assertRaisesRegex(ValueError, "missing compiler input"):
            self.compile("import missing.bend as Missing\n")

    def test_import_cycle(self):
        with self.assertRaisesRegex(ValueError, "cycle"):
            self.compile("import helper.bend as Helper\n", **{
                "helper.bend": "import main.bend as Main\n"
            })

    def test_hub_import_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "hub imports are unsupported"):
            self.compile("import foo@1.0.0.0/math.bend as Math\n")

    def test_import_cannot_escape_project(self):
        with self.assertRaisesRegex(ValueError, "escapes project"):
            self.compile("import ../secret.bend as Secret\n")


if __name__ == "__main__":
    unittest.main()
