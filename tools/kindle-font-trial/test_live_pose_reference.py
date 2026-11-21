import os
from pathlib import Path
import shlex
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

from assisted_shelf import device_program
from live_pose_reference import PROBE, SSH_LIMIT, WINDOW, decode_reference, measure, owned_window, reference_program
from run import transport


class ReferenceTests(unittest.TestCase):
    def observe(self, values):
        self.cues, self.references = [], []
        self.now = 0.0
        values = iter(values)

        def advance(amount):
            self.now += amount

        def scan(limit):
            self.assertGreater(limit, 0)
            self.assertLessEqual(limit, SSH_LIMIT)
            advance(min(limit, 1))
            return next(values, 'D:1')

        return measure(scan, self.cues.append, self.references.append, lambda: self.now, advance)

    def test_turn_then_two_consecutive_true_reads(self):
        self.assertEqual(self.observe(['D:1','U:3','D:1','U:3','U:3']), 'target_confirmed')
        self.assertEqual(self.cues, ['TURN_NOW', 'POSE D:1', 'POSE U:3', 'POSE D:1', 'POSE U:3'])
        self.assertEqual(len(self.references), 5)

    def test_two_initial_up_returns_early_without_turn(self):
        self.assertEqual(self.observe(['U:3','U:3']), 'target_confirmed')
        self.assertEqual(self.cues, ['HOLD_STILL','POSE U:3'])
        self.assertLess(self.now, WINDOW)

    def test_no_transition_expires_and_reads_are_bounded(self):
        self.assertEqual(self.observe([]), 'target_not_confirmed')
        self.assertAlmostEqual(self.now, WINDOW)
        self.assertEqual(self.cues, ['TURN_NOW','POSE D:1'])

    def test_unknown_reference_sends_no_action(self):
        with self.assertRaisesRegex(ValueError,'unsupported_reference'):
            self.observe(['unexpected'])
        self.assertEqual(self.cues, [])

    def test_fixed_is_an_observed_non_direction_and_breaks_two_up_reads(self):
        self.assertEqual(self.observe(['FIXED:3','U:3','FIXED:2','U:3','U:3']), 'target_confirmed')
        self.assertEqual(self.cues, ['HOLD_STILL','POSE FIXED:3','POSE U:3','POSE FIXED:2','POSE U:3'])
        self.assertEqual(self.references[0], 'FIXED:3')
        self.assertEqual(self.observe(['FIXED:0']*40), 'target_not_confirmed')

    def test_legal_non_target_rotations_and_any_initial_up_never_cue_a_turn(self):
        data = ['U:1','D:3','D:0','U:0','D:2','U:2','U:3','U:3']
        self.assertEqual(self.observe(data), 'target_confirmed')
        self.assertEqual(self.references, data)
        self.assertEqual(self.cues, ['HOLD_STILL', *('POSE '+pose for pose in data[:-1])])

    def test_missing_portrait_resets_consecutive_and_keeps_earlier_samples(self):
        self.assertEqual(self.observe([None, 'D:1', 'U:3', None, 'U:3', 'U:3']), 'target_confirmed')
        self.assertEqual(self.references, [None, 'D:1', 'U:3', None, 'U:3', 'U:3'])
        self.assertEqual(self.cues.count('TURN_NOW'), 1)

    def test_only_exact_pose_read_failure_is_retryable_after_both_guards(self):
        self.assertIsNone(decode_reference(SimpleNamespace(returncode=1, stdout=b'', stderr=b'refused pose_transient\n')))
        for code in ('U:0','D:0','U:1','D:1','U:2','D:2','U:3','D:3','FIXED:1'):
            self.assertEqual(decode_reference(SimpleNamespace(returncode=0, stdout=(code+'\n').encode(), stderr=b'')), code)
        for status, out, error in [(1,b'',b'refused protected_guard\n'), (1,b'',b'refused native_guard\n'),
                                    (255,b'',b'refused pose_transient\n'), (1,b'',b'foreign error\nrefused pose_transient\n'),
                                    (0,b'FIXED:1\n',b'unexpected transport stderr\n')]:
            with self.assertRaises(ValueError):
                decode_reference(SimpleNamespace(returncode=status, stdout=out, stderr=error))
        generated = reference_program(device_program())
        self.assertIn('reference) native || fail native_guard; state >/dev/null || fail protected_guard; portrait;;', generated)
        with self.assertRaisesRegex(ValueError, 'frozen'):
            reference_program('reference) portrait;;')
        with self.assertRaisesRegex(ValueError, 'frozen'):
            reference_program(device_program() + PROBE)

    def test_real_transport_shell_drives_probe_stderr_and_preserves_guards(self):
        with tempfile.TemporaryDirectory() as temporary:
            fake = Path(temporary)/'native-probe'
            marker = Path(temporary)/'called'
            fake.write_text('''#!/bin/sh
[ "$*" = '--kindle --device scribe --probe-native-portrait-calibration' ] || exit 64
touch "$MARKER"
case "$VARIANT" in
 changed) echo 'weread: device operation failed: The native portrait direction changed during calibration' >&2; exit 1;;
 frame_change) echo 'weread: device operation failed: The native framebuffer metadata changed during calibration' >&2; exit 1;;
 power) echo 'weread: device operation failed: The native power state changed during calibration' >&2; exit 1;;
 mixed) echo 'foreign diagnostic' >&2; echo 'weread: device operation failed: The native portrait direction changed during calibration' >&2; exit 1;;
 untrusted) echo 'weread: device operation failed: The native portrait direction changed during calibration' >&2; exit 69;;
 missing) exit 69;;
 invalid) echo 'FIXED:4';;
 *) echo 'FIXED:2';;
esac
''')
            fake.chmod(0o700)
            body = reference_program(device_program()).replace('/mnt/us/weread/app/weread --kindle --device scribe --probe-native-portrait-calibration',
                                                                shlex.quote(str(fake)) + ' --kindle --device scribe --probe-native-portrait-calibration')
            before, delimiter, after = body.rpartition('case "$mode" in\n    reference)')
            self.assertTrue(delimiter)
            body = before + 'native() { [ "$VARIANT" != native ]; }; state() { [ "$VARIANT" != protected ]; }\n' + delimiter + after
            command, header = transport(body, 'reference', '0123456789abcdef', 'none')
            for variant in ('fixed','changed','frame_change','native','protected','power','mixed','untrusted','missing','invalid'):
                with self.subTest(variant=variant):
                    marker.unlink(missing_ok=True)
                    raw = subprocess.run(['sh','-c',command], input=header, capture_output=True, timeout=5,
                                         env=dict(os.environ, VARIANT=variant, MARKER=str(marker)))
                    self.assertEqual(marker.exists(), variant not in ('native','protected'))
                    if variant in ('fixed','changed','frame_change'):
                        self.assertEqual(decode_reference(raw), 'FIXED:2' if variant == 'fixed' else None)
                    else:
                        expected = {'native':'native_guard','protected':'protected_guard','invalid':'reference_output_invalid'}.get(variant,'calibration_probe_failed')
                        with self.assertRaisesRegex(ValueError, expected):
                            decode_reference(raw)

    def test_final_short_deadline_missing_response_is_not_a_transport_pass(self):
        timer = [0.0]; output = []
        def read(limit):
            timer[0] += limit
            if limit < SSH_LIMIT:
                raise TimeoutError
            return 'D:1'
        self.assertEqual(measure(read, lambda _: None, output.append, lambda: timer[0], lambda t: timer.__setitem__(0,timer[0]+t)), 'target_not_confirmed')
        self.assertEqual(output, ['D:1']*3)
        with self.assertRaisesRegex(ValueError, 'reference_timeout'):
            measure(Mock(side_effect=TimeoutError), lambda _: None, output.append, lambda:0, lambda _:None)

    def test_all_owned_outcomes_are_released_before_return(self):
        with tempfile.TemporaryDirectory() as temporary:
            token = Path(temporary)/'token'
            request = SimpleNamespace(token_file=token, owner_thread='fixture', run_id='weread-test')
            for result, expected in [('target_confirmed',0), ('target_not_confirmed',2), (ValueError('reference_timeout'),1)]:
                lease = Mock()
                token.touch()
                lease.acquire.return_value = {'acquired_at_utc':'2026-11-21T02:00:00+00:00'}
                lease.release.return_value = dict(ok=True, operation='release', state='released', project='WeRead', owner_thread='fixture',
                                                  run_id='weread-test', owner_and_token_checked=True, only_owned_owner_json_removed_then_rmdir=True,
                                                  native_restored_and_connections_closed=True, lease_directory_absent_after_release=True,
                                                  released_at_utc='2026-11-21T02:00:01+00:00')
                observer = Mock(side_effect=result) if isinstance(result, Exception) else Mock(return_value=result)
                report = {}
                self.assertEqual(owned_window(lease, request, report, lambda: None, observer), expected)
                self.assertFalse(report['recovery_required'] or token.exists())
                self.assertTrue(report['owner_and_token_checked'])
                lease.release.assert_called_once_with(request, native_restored_and_connections_closed=True)

    def test_bad_release_receipt_or_exception_does_not_promote_positive_pose_or_remove_token(self):
        with tempfile.TemporaryDirectory() as temporary:
            token = Path(temporary)/'private-token'; token.touch()
            request = SimpleNamespace(token_file=token, owner_thread='fixture', run_id='weread-test')
            good = dict(ok=True, operation='release', state='released', project='WeRead', owner_thread='fixture', run_id='weread-test',
                        owner_and_token_checked=True, only_owned_owner_json_removed_then_rmdir=True, lease_directory_absent_after_release=True,
                        native_restored_and_connections_closed=True, released_at_utc='2026-11-21T02:00:01+00:00')
            for change in ({'owner_thread':'other'}, {'ok':1}, {'native_restored_and_connections_closed':False},
                           {'released_at_utc':'2026-11-21T01:59:59+00:00'}, {}, {'lease_directory_absent_after_release':'no'}):
                lease = Mock()
                lease.acquire.return_value = {'acquired_at_utc':'2026-11-21T02:00:00+00:00'}
                if change:
                    lease.release.return_value = good | change
                else:
                    lease.release.side_effect = RuntimeError('unavailable')
                report = {}
                self.assertEqual(owned_window(lease,request,report,lambda: None,lambda: 'target_confirmed'), 1)
                self.assertTrue(report['recovery_required'] and token.exists())
                self.assertNotIn('released', report)
                self.assertTrue(report['release_error'])

    def test_busy_or_unconfirmed_acquisition_never_connects_or_releases(self):
        with tempfile.TemporaryDirectory() as temporary:
            lease, observer = Mock(), Mock()
            lease.acquire.side_effect = RuntimeError('busy')
            report = {}
            self.assertEqual(owned_window(lease, SimpleNamespace(token_file=Path(temporary)/'token'), report, lambda: None, observer), 1)
            self.assertFalse(report['recovery_required'])
            observer.assert_not_called()
            lease.release.assert_not_called()


if __name__ == '__main__':
    unittest.main()
