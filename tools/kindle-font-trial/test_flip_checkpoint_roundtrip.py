"""Pass the device collector's own synthetic Linux archives to the shipped analyzer unchanged."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from analyze_pose_flip import CASE, SEQUENCE, analyze
from prepare_pose_flip import DRAW_CONTRACT
import test_pose_flip_device as device


class DeviceToReport(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = tempfile.TemporaryDirectory(); cls.addClassCleanup(cls.scratch.cleanup)
        shim = device.SHIM.replace('value->tv_sec = 1;', '''
    unsigned long long position = 99995000000ULL + (clock_reads - 1) / 2 * 50000001ULL + ((clock_reads - 1) % 2);
    value->tv_sec = position / 1000000000ULL;''').replace(
            'value->tv_nsec = variant("reversed") ? 9 - clock_reads : clock_reads;', 'value->tv_nsec = position % 1000000000ULL;').replace(
            '15 + sampled % 4', '(sampled < atoi(getenv("TURN_AT")) ? 16 : 15)')
        with patch.object(device, 'SHIM', shim):
            device.FlipDeviceTests.setUpClass()
        cls.addClassCleanup(device.FlipDeviceTests.doClassCleanups)

    def collect(self, expected, case_reason=None):
        scenario = device.FlipDeviceTests('test_frozen_shell_and_sensor_child_join_before_closed_public_archive')
        scenario.setUp(); self.addCleanup(scenario.doCleanups)
        fixture = scenario.fixture; fixture.env.update(NATIVE='D:1',LAUNCH_POSE='U:3',TURN_AT='2' if expected=='hit' else '40')
        if case_reason: fixture.env.update(FINAL_VARIANT='missing')
        fixture.stage_visit()
        measure = fixture.stage / 'measure'
        original = measure.read_text(); self.assertEqual(original.count('REQUEST 122.000000000'),1)
        measure.write_text(original.replace('REQUEST 122.000000000','REQUEST 100.000000000'))
        result = fixture.step('case',CASE,check=not case_reason)
        if case_reason: self.assertIn(('refused '+case_reason).encode(), result.stderr)
        fixture.step('restore')
        path = Path(self.scratch.name)/'collector-output.tgz'; path.write_bytes(fixture.step('collect').stdout)
        fixture.step('close'); fixture.step('absent')
        label = device.shelf_fixture.LABEL
        receipt=dict(label=label, purpose=DRAW_CONTRACT['purpose'], scope='transaction_only', passed=not bool(case_reason),
                     owner_token_checked_on_release=True, acquired='2026-11-21T03:01:01Z', operator_cue_issued='2026-11-21T03:01:02Z',
                     operator_cue_acknowledged='2026-11-21T03:01:03Z',released='2026-11-21T03:02:01Z',initial_native_reference='D:1',operations=[
                         dict(mode=mode,case=case,status=result.returncode if mode=='case' else 0,
                              at='2026-11-21T03:01:01Z' if mode=='stage' else '2026-11-21T03:01:50Z',
                              diagnostic='refused '+case_reason if case_reason and mode=='case' else '') for mode,case in SEQUENCE])
        report=analyze(DRAW_CONTRACT|dict(label=label),receipt,path)
        self.assertEqual(report['startup_turn']['coverage'],expected)
        self.assertEqual(report['first_application_draw']['renderer_rotation_degrees'],180)
        self.assertEqual(report['first_application_draw']['turn_relative_to_draw'], 'before_draw' if expected=='hit' else 'after_draw')
        self.assertEqual(report['launch_calibration']['comparison_to_before'],'different')
        self.assertEqual(report['touch'],'not_tested')
        if case_reason:
            self.assertEqual(report['collection'],'pose_checkpoint_only_released')
            self.assertIsNone(report['native_return'])
        else:
            self.assertEqual(report['collection'],'completed_and_released')

    def test_hit_survives_missing_final_metadata_and_an_opposite_launch_anchor(self):
        self.collect('hit','capture_metadata')

    def test_late_turn_with_a_different_launch_anchor_is_still_a_miss(self):
        self.collect('missed')


if __name__ == '__main__': unittest.main()
