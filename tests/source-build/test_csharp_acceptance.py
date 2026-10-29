"""Acceptance consumers keep compiler diagnostics separate from their result."""
import os
from pathlib import Path
import sys
import tempfile
import unittest

from csharp import logged_command


class ConsumerEvidenceTests(unittest.TestCase):
    def test_success_with_compiler_warnings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            status, result = logged_command(
                [sys.executable, '-c', 'import sys; print("warning: unused field", file=sys.stderr); print("consumer-ok")'],
                cwd=root, environment=os.environ.copy(), stdout=root / 'stdout', stderr=root / 'stderr')
            self.assertEqual(status, 0)
            self.assertEqual(result, 'consumer-ok')
            self.assertEqual((root / 'stderr').read_text(), 'warning: unused field\n')

    def test_failure_keeps_exit_status_and_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            status, result = logged_command(
                [sys.executable, '-c', 'import sys; print("assertion failed", file=sys.stderr); sys.exit(7)'],
                cwd=root, environment=os.environ.copy(), stdout=root / 'stdout', stderr=root / 'stderr')
            self.assertEqual(status, 7)
            self.assertEqual(result, '')
            self.assertIn('assertion failed', (root / 'stderr').read_text())
