import fcntl
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from pose_probe import COUNT, parse_observation, render_device


LABEL = "1234567890abcdef"
DISPATCH = 'case "$mode" in\n    admit)'
HELPER = b'''#!/bin/sh
[ "$1" = 10 ] || exit 64
for index in $(seq 1 10); do printf '%s %s %s 16\n' "$index" "$((100*index))" "$((100*index+1))"; done
'''


class PoseFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.stage = self.root / ("weread-pose-probe-" + LABEL)
        self.before = "D:1"
        self.after = "D:1"

    def execute(self, mode, payload=HELPER, expected=0, extra="", reason=None, length=None, *, display_trace=False):
        source = render_device(len(payload) if length is None else length, display_trace=display_trace)
        self.assertIn('file_identity /mnt/us/bilicomics/app/bilicomics 74340832', source)
        for excluded in ('sha256sum ', 'lipc-set-prop', 'rm -rf', 'scan() {'):
            self.assertNotIn(excluded, source)
        source = source.replace('tmp=$(readlink -f /tmp) || exit 1', f'tmp={self.root}').replace(
            'case "$tmp" in /tmp|/var/tmp)', f'case "$tmp" in {self.root})')
        source = source.replace('home=/mnt/us/weread', f'home={self.root}/home')
        helper = self.root / 'home/app/weread'
        helper.parent.mkdir(parents=True, exist_ok=True)
        helper.write_text(f'''#!/bin/sh
[ "$*" = '--kindle --device scribe --probe-native-portrait-calibration' ] || exit 1
if [ -f {self.root}/second ]; then printf '%s\\n' '{self.after}';
else touch {self.root}/second; printf '%s\\n' '{self.before}'; fi
''')
        helper.chmod(0o700)
        mocks = 'state() { echo native-stable; }\nview() { echo kpp_library; }\npeer_projection() { echo peers-stable; }\n'
        source = source.replace(DISPATCH, mocks + extra + '\n' + DISPATCH)
        child = subprocess.run(['sh', '-c', source, 'pose-test', mode, LABEL, 'none'], input=payload,
                               capture_output=True, timeout=8 if display_trace else 5)
        self.assertEqual(child.returncode, expected, child.stderr)
        if reason:
            self.assertEqual(child.stderr, f'refused {reason}\n'.encode())
        return child.stdout


class ProbeTests(PoseFixture):
    def test_self_cleaning_probe_and_independent_absence(self):
        self.assertEqual(self.execute('admit'), b'admitted\n')
        self.assertFalse(self.stage.exists())
        receipt = parse_observation(self.execute('probe'), LABEL)
        self.assertEqual(receipt['classification'], 'stable_native_reference')
        self.assertEqual(receipt['native_before'], receipt['native_after'])
        self.assertEqual(receipt['samples'][-1], [COUNT, 1000, 1001, 16])
        self.assertFalse(self.stage.exists())
        self.assertEqual(self.execute('close'), b'closed\n')
        self.assertEqual(self.execute('absent'), b'absent\n')

    def test_native_inactivity_peer_drift_and_incomplete_upload_refuse_without_leaks(self):
        self.execute('admit', extra='state() { return 10; }', expected=1, reason='not_native_idle')
        self.execute('admit', extra='peer_projection() { return 1; }', expected=1, reason='peer_or_registration_guard')
        self.execute('probe', length=len(HELPER) + 1, expected=1, reason='upload')
        self.execute('probe', payload=b'#!/bin/sh\nexit 2\n', expected=1, reason='sensor_unavailable')
        self.execute('probe', extra='peer_projection() { if [ -e "$stage" ]; then echo drift; else echo initial; fi; }',
                     expected=1, reason='protected_drift')
        self.assertFalse(self.stage.exists())

    def test_pose_changes_do_not_block_cleanup_or_claim_stability(self):
        self.after = 'U:3'
        row = parse_observation(self.execute('probe'), LABEL)
        self.assertEqual((row['classification'], row['reason']), ('inconclusive', 'native_observations_differ'))
        self.assertFalse(self.stage.exists())
        (self.root / 'second').unlink()
        self.before = self.after = 'FIXED:3'
        row = parse_observation(self.execute('probe'), LABEL)
        self.assertEqual(row['reason'], 'native_direction_not_available')

    def test_refuse_foreign_contents_and_close_only_after_lock_owner_exits(self):
        self.stage.mkdir(mode=0o700)
        owner = self.stage / 'owner'
        owner.write_text('other\n'); owner.chmod(0o600)
        self.execute('close', expected=1, reason='cleanup')
        self.assertEqual(owner.read_text(), 'other\n')
        owner.write_text(LABEL + '\n')
        sentinel = self.root / 'other'
        sentinel.write_text('unchanged')
        for name in ('snapshot', '.unrecognized'):
            foreign = self.stage / name
            foreign.symlink_to(sentinel)
            self.execute('close', expected=1, reason='cleanup')
            self.assertTrue(foreign.is_symlink())
            foreign.unlink()
        descriptor = os.open(self.stage, os.O_RDONLY | os.O_DIRECTORY)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.execute('close', expected=1, reason='cleanup')
        finally:
            os.close(descriptor)
        self.execute('close')
        self.assertEqual(sentinel.read_text(), 'unchanged')
        self.assertFalse(self.stage.exists())

    def test_parser_rejects_unbound_missing_extended_or_disordered_records(self):
        output = self.execute('probe').decode()
        for invalid in [output.replace(LABEL, '0' * 16), output.replace('1 100 101 16\n', ''),
                        output.replace('2 200 201 16', '2 99 201 16'), output.replace('3 300 301 16', '3 302 301 16'),
                        output.replace('4 400 401 16', '4 400 401 99'), output + 'unreviewed data\n', output.rstrip()]:
            with self.assertRaises(ValueError):
                parse_observation(invalid, LABEL)
        self.assertEqual(parse_observation(output.replace('101 16', '101 15'), LABEL)['reason'], 'sensor_directions_differ')
        self.assertEqual(parse_observation(output.replace('native_before D:1', 'native_before U:3').replace(
            'native_after D:1', 'native_after U:3'), LABEL)['reason'], 'native_sensor_disagreement')


if __name__ == '__main__':
    unittest.main()
