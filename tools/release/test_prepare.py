"""Exercise native release packaging without downloading or compiling tools."""
import importlib.util
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('prepare_release', Path(__file__).with_name('prepare.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class NativeReleaseTests(unittest.TestCase):
    def test_kit_preserves_layout_empty_caches_and_internal_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prepared = root / 'prepared'
            (prepared / 'bin').mkdir(parents=True)
            (prepared / 'bin/wisp').write_bytes(b'cli')
            (prepared / 'bin/wisp-worker').write_bytes(b'worker')
            (prepared / 'bin/alias').symlink_to('wisp')
            (prepared / 'cache').mkdir()
            (prepared / 'native-env.json').write_text(json.dumps({'CARGO_HOME': 'cache'}))
            archive = root / 'native.tar.gz'
            value = release.package(prepared, archive, 'https://example.com/native.tar.gz')
            self.assertEqual(value['runtime']['sha256'], release.digest(archive))
            self.assertEqual(value['env'], {'CARGO_HOME': 'cache'})
            with tarfile.open(archive) as source:
                self.assertTrue(source.getmember('cache').isdir())
                self.assertTrue(source.getmember('bin/alias').isfile())
                self.assertEqual(source.extractfile('bin/alias').read(), b'cli')

    def test_kit_never_follows_external_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prepared = root / 'prepared'
            prepared.mkdir()
            (root / 'private').write_bytes(b'private')
            (prepared / 'link').symlink_to(root / 'private')
            with self.assertRaisesRegex(ValueError, 'escapes'):
                release.package(prepared, root / 'native.tar.gz', 'https://example.com/native.tar.gz')


if __name__ == '__main__':
    unittest.main()
