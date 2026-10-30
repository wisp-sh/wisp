import unittest
from acquire import REQUIRED_STEPS, validate


class CompilerEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.pin = {'repository': 'wisp-sh/wisp-buildtools', 'commit': 'a' * 40,
                    'workflow': '.github/workflows/ci.yml', 'artifact': 'python-linux-x86_64'}
        self.run = {'head_sha': self.pin['commit'], 'conclusion': 'success', 'status': 'completed',
                    'path': self.pin['workflow'], 'repository': {'full_name': self.pin['repository']}, 'html_url': 'run'}
        self.jobs = {'jobs': [{'name': 'Build python / python', 'conclusion': 'success', 'html_url': 'job',
                              'steps': [{'name': step, 'conclusion': 'success'} for step in REQUIRED_STEPS]}]}
        self.artifacts = {'artifacts': [{'name': self.pin['artifact'], 'expired': False, 'digest': 'sha256:' + 'b' * 64}]}

    def test_accepts_executed_pinned_compiler(self):
        self.assertEqual(validate(self.pin, self.run, self.jobs, self.artifacts)['pin'], self.pin)

    def test_green_summary_with_skipped_compiler_is_rejected(self):
        for step in self.jobs['jobs'][0]['steps']:
            step['conclusion'] = 'skipped'
            with self.assertRaisesRegex(ValueError, 'did not all execute'):
                validate(self.pin, self.run, self.jobs, self.artifacts)
            step['conclusion'] = 'success'

    def test_stale_source_and_expired_artifact_are_rejected(self):
        self.run['head_sha'] = 'c' * 40
        with self.assertRaisesRegex(ValueError, 'pinned source'):
            validate(self.pin, self.run, self.jobs, self.artifacts)
        self.run['head_sha'] = self.pin['commit']
        self.artifacts['artifacts'][0]['expired'] = True
        with self.assertRaisesRegex(ValueError, 'unexpired'):
            validate(self.pin, self.run, self.jobs, self.artifacts)


if __name__ == '__main__':
    unittest.main()
