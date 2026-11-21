from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import run_pose_probe as runner
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

        def parse(_, *, expected_label, display_trace):
            self.assertEqual(expected_label, self.report['label'])
            self.assertFalse(display_trace)
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


class DisplayTraceHostTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.helper = self.root / 'helper'
        header = bytearray(52)
        header[:7] = b'\x7fELF\x01\x01\x01'
        header[18:20] = (40).to_bytes(2, 'little')
        self.helper.write_bytes(header)
        self.lease = Mock()
        self.lease.LeaseRequest.side_effect = lambda _, path, owner, tag: SimpleNamespace(token_file=path, owner=owner, tag=tag)
        self.lease.acquire.side_effect = self.acquire
        self.lease.status.return_value = {'ownership_verified': True}
        self.lease.release.return_value = {'owner_and_token_checked': True, 'only_owned_owner_json_removed_then_rmdir': True,
                                          'released_at_utc': 'fixture', 'lease_directory_absent_after_release': True}
        self.calls = []
        self.timings = {}
        self.prompt_seen = {}
        self.feature = True
        self.failure = None
        self.credential = 'test-only'

    def acquire(self, request):
        if self.failure == 'acquire':
            raise ValueError('test busy')
        request.token_file.write_text('fixture')
        return {'acquired_at_utc': 'fixture'}

    def device(self, command, *, input, timeout, **_):
        operation = command[-1].split()[-3]
        self.calls.append(operation)
        self.timings[operation] = timeout
        self.prompt_seen[operation] = 'trace is about to start' in self.output.getvalue()
        if operation == self.failure == 'probe':
            raise subprocess.TimeoutExpired(command, timeout)
        if operation == self.failure:
            return subprocess.CompletedProcess(command, 1, b'', b'refused fixture\n')
        if operation == 'probe':
            self.assertTrue(input.endswith(self.helper.read_bytes()))
        result = dict(admit=b'admitted\n', probe=b'simulated receipt\n', close=b'closed\n', absent=b'absent\n')
        return subprocess.CompletedProcess(command, 0, result[operation], b'')

    def parse(self, text, *, expected_label, display_trace):
        self.assertTrue(self.lease.release.called)
        self.assertEqual(text, 'simulated receipt\n')
        self.assertRegex(expected_label, '^[0-9a-f]{16}$')
        self.assertEqual(display_trace, self.feature)
        if display_trace:
            return {'classification': 'display_diagnostic_only', 'app_admitted': False, 'protected_state_unchanged': True}
        return {'classification': 'stable_native_reference', 'protected_state_unchanged': True}

    def execute(self, *, preflight=False, recover=None):
        self.output = io.StringIO()
        args = SimpleNamespace(display_trace=self.feature, preflight=preflight, recover=recover,
                               helper=self.helper, lease_tool=self.helper, owner='test-owner', runs=self.root)
        render = Mock(return_value='exit 1\n')
        regular = Path.is_file
        with patch.object(Path, 'is_file', lambda path: str(path) == '/opt/kindle-toolchain/ubuntu-run' or regular(path)), \
             patch.dict(runner.UTIL, load_lease=lambda *_: self.lease, ssh_command=lambda *_: ['simulated-ssh']), \
             patch.object(runner, 'render_device', render), patch.object(runner, 'parse_observation', side_effect=self.parse), \
             patch.object(runner.sys, 'stdin', io.StringIO(self.credential + '\n')), patch.object(runner.signal, 'signal'), \
             patch.object(runner.subprocess, 'run', side_effect=self.device), redirect_stdout(self.output):
            status = runner.main(args)
        if recover:
            render.assert_not_called()
        else:
            render.assert_called_once_with(self.helper.stat().st_size, display_trace=self.feature)
        return status

    def attempt(self):
        directory, = self.root.glob('probe-*')
        return directory

    def test_trace_reports_the_scope_and_only_announces_after_authenticated_admission(self):
        self.assertEqual(self.execute(), 0)
        self.assertEqual(self.calls, ['admit', 'probe', 'close', 'absent'])
        self.assertFalse(self.prompt_seen['admit'])
        self.assertTrue(self.prompt_seen['probe'])
        self.assertEqual(self.timings, {'admit': 30, 'probe': 40, 'close': 30, 'absent': 30})
        report = json.loads((self.attempt() / 'operations.json').read_text())
        self.assertEqual((report['mode'], report['scope']), ('display_trace', 'stock_native_display_trace_only'))
        self.assertTrue(report['passed'] and report['owner_and_token_checked'])
        self.assertEqual(report['observation']['classification'], 'display_diagnostic_only')
        self.assertIs(report['observation']['app_admitted'], False)
        self.assertNotIn('turn_confirmed', report['observation'])
        self.assertNotIn('TURN_NOW', self.output.getvalue())

    def test_normal_probe_retains_its_separate_short_scope_and_no_cue(self):
        self.feature = False
        self.assertEqual(self.execute(), 0)
        report = json.loads((self.attempt() / 'operations.json').read_text())
        self.assertEqual((report['mode'], report['scope']), ('probe', 'stock_native_reference_only'))
        self.assertEqual(self.timings['probe'], 30)
        self.assertFalse(any(self.prompt_seen.values()))

    def test_frozen_stationary_display_output_is_a_valid_transaction_not_a_turn_or_admission(self):
        report = {'label': '0123456789abcdef'}
        wire = (b'pose_display_trace 0123456789abcdef\n'
                b'read 1 100 110 16 1 D 200 210 16 1\ntrace_end budget\nprotected_state unchanged\n')
        steps = []

        def call(operation, expected):
            steps.append(operation)
            return wire if operation == 'probe' else expected

        def release():
            steps.append('release')
            return self.lease.release.return_value

        transact(call, release, report, lambda: None, display_trace=True)
        self.assertEqual(steps, ['admit', 'probe', 'close', 'absent', 'release'])
        self.assertTrue(report['passed'])
        self.assertEqual(report['observation']['trace_end'], 'budget')
        self.assertEqual(report['observation']['classification'], 'display_diagnostic_only')
        self.assertIs(report['observation']['app_admitted'], False)
        self.assertNotIn('turn_confirmed', report['observation'])

    def test_preflight_or_missing_auth_or_refused_acquire_cannot_signal_a_hold(self):
        self.credential = ''
        self.assertEqual(self.execute(preflight=True), 0)
        self.lease.acquire.assert_not_called()
        self.lease.LeaseRequest.assert_not_called()
        with self.assertRaisesRegex(ValueError, 'Missing private authentication'):
            self.execute()
        self.lease.acquire.assert_not_called()
        self.credential = 'test-only'
        self.failure = 'acquire'
        self.assertEqual(self.execute(), 1)
        self.assertFalse(self.calls)
        self.assertNotIn('trace is about to start', self.output.getvalue())

    def test_refused_native_admit_does_not_signal_or_issue_probe(self):
        self.failure = 'admit'
        self.assertEqual(self.execute(), 1)
        self.assertEqual(self.calls, ['admit'])
        self.lease.release.assert_called_once()
        self.assertNotIn('trace is about to start', self.output.getvalue())

    def test_timeout_cleans_and_releases_without_claiming_an_observation(self):
        self.failure = 'probe'
        self.assertEqual(self.execute(), 1)
        report = json.loads((self.attempt() / 'operations.json').read_text())
        self.assertEqual(self.calls, ['admit', 'probe', 'close', 'absent'])
        self.assertEqual(report['operations'][1]['reason'], 'transport_timeout')
        self.assertNotIn('observation', report)
        self.assertTrue(report['released'])

    def test_recovery_cannot_select_trace_and_only_cleans_the_original_stage(self):
        self.failure = 'absent'
        self.assertEqual(self.execute(), 1)
        self.lease.release.assert_not_called()
        attempt = self.attempt()
        with self.assertRaisesRegex(ValueError, 'cleanup only'):
            self.execute(recover=attempt)
        self.calls.clear()
        self.feature = False
        self.failure = None
        self.assertEqual(self.execute(recover=attempt), 0)
        report = json.loads((attempt / 'recovery.json').read_text())
        self.assertEqual((report['mode'], report['scope']), ('recover', 'stage_cleanup_only'))
        self.assertNotIn('observation', report)
        self.assertEqual(self.calls, ['close', 'absent'])
        self.assertFalse(any(self.prompt_seen[op] for op in self.calls))
        self.lease.acquire.assert_called_once()


if __name__ == '__main__':
    unittest.main()
