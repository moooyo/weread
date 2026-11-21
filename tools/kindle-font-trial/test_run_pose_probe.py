import unittest
from unittest.mock import patch

from run_pose_probe import transact


class WorkerPoseProbeTests(unittest.TestCase):
    def execute(self, failed=None, recovering=False, refuse_release=False):
        self.operations = []
        self.report = {'label': '0123456789abcdef'}

        def call(mode, expected):
            self.operations.append(mode)
            if mode == failed:
                raise ValueError('rejected ' + mode)
            return b'fixture' if expected is None else expected

        def release():
            self.operations.append('release')
            return {'owner_and_token_checked': not refuse_release, 'only_owned_owner_json_removed_then_rmdir': True,
                    'released_at_utc': 'fixture', 'lease_directory_absent_after_release': True}

        def parse(_, *, expected_label):
            self.assertEqual(expected_label, self.report['label'])
            self.operations.append('analyze')
            if failed == 'analyze':
                raise ValueError('malformed observation')
            return {'fixture': True}

        with patch('run_pose_probe.parse_observation', side_effect=parse):
            transact(call, release, self.report, lambda: None, recovering)

    def test_analysis_is_after_close_absence_and_release(self):
        self.execute()
        self.assertEqual(self.operations, ['admit', 'probe', 'close', 'absent', 'release', 'analyze'])
        self.assertTrue(self.report['passed'])

    def test_read_only_admission_refusal_does_not_stage(self):
        with self.assertRaises(ValueError):
            self.execute('admit')
        self.assertEqual(self.operations, ['admit', 'release'])

    def test_failed_probe_or_parser_does_not_hold_the_device(self):
        for mode in ('probe', 'analyze'):
            with self.assertRaises(ValueError):
                self.execute(mode)
            self.assertTrue(self.report['released'])
            self.assertNotIn('passed', self.report)

    def test_uncertain_cleanup_does_not_release(self):
        for mode in ('close', 'absent'):
            with self.assertRaises(ValueError):
                self.execute(mode)
            self.assertNotIn('release', self.operations)

    def test_recovery_is_close_only_and_refuses_unchecked_release(self):
        self.execute(recovering=True)
        self.assertEqual(self.operations, ['close', 'absent', 'release'])
        with self.assertRaises(ValueError):
            self.execute(recovering=True, refuse_release=True)
        self.assertNotIn('released', self.report)


if __name__ == '__main__':
    unittest.main()
