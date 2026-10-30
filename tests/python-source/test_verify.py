import copy
import unittest
from verify import REQUIRED_CHECKS, validate


class InstalledEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.commit = 'a' * 40
        self.pin = {'commit': 'b' * 40, 'repository': 'wisp-sh/wisp-buildtools'}
        self.result = {'wispCommit': self.commit, 'toolchainPin': self.pin, 'passed': True,
                       'complete': True, 'cleanInstallVerified': True, 'checks': sorted(REQUIRED_CHECKS)}
        self.acquisition = {'pin': self.pin}
        self.before = {'valid': True, 'packages': [{'name': 'python', 'sha256': 'c' * 64}]}
        self.after = copy.deepcopy(self.before)

    def validate(self):
        validate(self.result, self.acquisition, self.pin, self.commit, self.before, self.after)

    def test_complete_matching_acceptance(self):
        self.validate()

    def test_stale_wisp_or_compiler_evidence(self):
        self.result['wispCommit'] = 'd' * 40
        with self.assertRaisesRegex(ValueError, 'commit or compiler pin'):
            self.validate()
        self.result['wispCommit'] = self.commit
        self.acquisition['pin'] = {'commit': 'e' * 40}
        with self.assertRaisesRegex(ValueError, 'commit or compiler pin'):
            self.validate()

    def test_green_partial_or_omitted_checks_are_rejected(self):
        for key in ('passed', 'complete', 'cleanInstallVerified'):
            self.result[key] = False
            with self.assertRaisesRegex(ValueError, 'did not complete'):
                self.validate()
            self.result[key] = True
        for check in REQUIRED_CHECKS:
            self.result['checks'] = sorted(REQUIRED_CHECKS - {check})
            with self.assertRaisesRegex(ValueError, 'omitted'):
                self.validate()

    def test_invalid_or_changed_final_inventory_is_rejected(self):
        self.after['valid'] = False
        with self.assertRaisesRegex(ValueError, 'inventory'):
            self.validate()
        self.after['valid'] = True
        self.after['packages'][0]['sha256'] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'inventory'):
            self.validate()
