import io
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import unittest

import pose_flip_device as flip
import test_assisted_shelf as shelf_fixture
from test_orientation import records
from test_orientation_device import archive
from test_pose_snapshot import SHIM


HERE = Path(__file__).resolve().parent
WRAPS = ['-Wl,--wrap=' + name for name in ('open', 'ioctl', 'close', 'fstat', 'lstat', 'clock_gettime', 'nanosleep')]
PAUSE_SHIM = '''int __wrap_nanosleep(const struct timespec *requested, struct timespec *remaining) {
    (void)remaining;
    if (requested->tv_sec || requested->tv_nsec != 50000000) _exit(77);
    usleep(2500);
    return 0;
}
'''


class SensorProgram(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temporary.cleanup)
        root = Path(cls.temporary.name)
        shim = root / 'fake-evdev.c'
        shim.write_text('#define _DEFAULT_SOURCE\n' + SHIM + PAUSE_SHIM)
        cls.program = root / 'sensor'
        subprocess.run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', str(HERE / 'pose_snapshot.c'), str(shim),
                        *WRAPS, '-o', cls.program], check=True)

    def test_policy_and_ready_from_first_real_snapshot(self):
        self.assertEqual(subprocess.check_output([self.program, '--flip-policy']),
                         b'startup-flip 401 50000000 27000000000 CLOCK_MONOTONIC\n')
        result = subprocess.run([self.program, '--startup-flip'], capture_output=True, env=dict(os.environ, POSE_TEST='kx132'), timeout=4)
        self.assertEqual((result.returncode, result.stderr), (0, b''))
        ready, *samples = result.stdout.splitlines()
        self.assertEqual(ready.split()[1:], samples[0].split()[1:])
        self.assertEqual(len(samples), flip.SAMPLES)
        self.assertEqual([int(row.split()[3]) for row in samples], [15 + index % 4 for index in range(flip.SAMPLES)])

    def test_invalid_discovery_never_signals_ready_and_changed_source_never_publishes_rows(self):
        for variant, lines in [('ambiguous', 0), ('wrong_name', 0), ('read_error', 1), ('replaced', 1)]:
            with self.subTest(variant=variant):
                result = subprocess.run([self.program, '--startup-flip'], capture_output=True, timeout=4, env=dict(os.environ, POSE_TEST=variant))
                self.assertEqual((result.returncode, len(result.stdout.splitlines())), (2, lines))


class FlipDeviceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        SensorProgram.setUpClass.__func__(cls)

    def setUp(self):
        self.fixture = shelf_fixture.AssistedDeviceTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        root = self.fixture.root
        self.out = self.fixture.out
        boot = root / 'proc/sys/kernel/random/boot_id'
        boot.parent.mkdir(parents=True)
        boot.write_text('11111111-1111-1111-1111-111111111111\n')
        source = flip.device_program(6)
        source = re.sub(r'/mnt/us|/sys/class/input|/proc/|/var/local|/tmp(?=[/)])', lambda match: str(root) + match[0], source)
        source = source.replace(str(root) + '/proc/$flip_child/', '/proc/$flip_child/')
        self.fixture.definitions, dispatch = source.split(self.fixture.dispatch.split('reference)')[0] + 'reference)', 1)
        self.fixture.dispatch = 'case "$mode" in\n    reference)' + dispatch
        self.fixture.mocks += '\nusleep() { sleep_fraction=0.005; /bin/sleep "$sleep_fraction"; }\n'
        with tarfile.open(fileobj=io.BytesIO(self.fixture.payload)) as payload:
            contents = {entry.name: payload.extractfile(entry).read() for entry in payload}
        log = self.fixture.stage / 'B/data/logs/application-bootstrap.log'
        (root / 'proc/60/fd/2').symlink_to(log)
        _, markers = records('B-library', 100)
        markers = markers.replace('WEREAD_STARTUP 100.120000000 first_application_frame_send',
                                  'WEREAD_STARTUP 100.115000000 first_application_frame_draw_begin_rot180\n'
                                  'WEREAD_STARTUP 100.120000000 first_application_frame_send')
        measure = contents['measure'].decode()
        measure = measure.replace('echo \'WEREAD_STARTUP 123.000000000 first_application_frame_send\'',
                                  'printf \'%s\' \'' + markers + '\'')
        before, send = markers.rsplit('WEREAD_STARTUP ', 1)
        full_log = f'printf \'%s\' \'{markers}\' >"$stage/B/data/logs/application-bootstrap.log"'
        staged_log = f'''log=$stage/B/data/logs/application-bootstrap.log
if [ -n "${{SPLIT_SEND:-}}" ]; then
    printf '%s' '{before}' >"$log"
    (for tick in $(seq 1 200); do [ ! -s "$out/sensor-binding" ] || break; /bin/sleep 0.005; done
     if [ "${{SPLIT_SEND}}" = expired ]; then /bin/sleep 1.6; fi
     if [ "${{SPLIT_SEND}}" = owner ]; then echo changed >"$ROOT/tmp/weread-launcher.lock/application-owned"; fi
     printf '%s' 'WEREAD_STARTUP {send}' >>"$log") </dev/null >/dev/null 2>&1 8>&- &
else
    {full_log}
fi'''
        self.assertIn(full_log, measure)
        measure = measure.replace(full_log, staged_log).replace('echo 0 >"$out/status"',
            'if [ -n "${SPLIT_SEND:-}" ]; then (while ! grep -q first_application_frame_send "$log"; do /bin/sleep 0.01; done; echo 0 >"$out/status") </dev/null >/dev/null 2>&1 8>&- & else echo 0 >"$out/status"; fi')
        measure += '''
anchor=$ROOT/tmp/weread-launcher.lock/native-portrait-calibration
case "${ANCHOR_VARIANT:-normal}" in
    missing) rm "$anchor";;
    malformed) printf 'private-unrecognized-value\\n' >"$anchor";;
    foreign_generation) printf '2:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:U:3\\n' >"$anchor";;
    unsafe) rm "$anchor"; ln -s /dev/null "$anchor";;
esac
case "${FINAL_VARIANT:-normal}" in
    missing) rm "$out/final.metadata.json";;
    empty) : >"$out/final.metadata.json";;
esac
if [ "${ROLL_AFTER_LOG:-}" = 1 ]; then
    (for tick in $(seq 1 200); do [ ! -s "$out/log-check" ] || break; /bin/sleep 0.005; done
     echo changed >"$ROOT/tmp/weread-launcher.lock/application-owned") </dev/null >/dev/null 2>&1 8>&- &
fi
'''
        contents.update({'pose-snapshot': self.program.read_bytes(), 'measure': measure.encode()})
        self.fixture.payload = archive(contents.items())
        self.fixture.env.update(POSE_TEST='kx132', POSE_DRAW_FIXTURE='1', POSE_ROTATE='1')

    def test_frozen_shell_and_sensor_child_join_before_closed_public_archive(self):
        self.fixture.stage_visit()
        self.fixture.step('case', flip.CASE)
        package = self.fixture.collect()
        self.assertEqual(set(package), {'out/orientation-batch', *('out/B-library/' + leaf for leaf in flip.LEAVES)})
        self.assertEqual(package['out/B-library/app-binding'], package['out/B-library/sensor-binding'])
        self.assertIn(b' B-library 11111111-1111-1111-1111-111111111111 60 55 CLOCK_MONOTONIC\n', package['out/B-library/app-binding'])
        self.assertEqual(len(package['out/B-library/sensor'].splitlines()), flip.SAMPLES)
        self.assertEqual(package['out/B-library/observation'], b'captured startup_turn\n')
        self.fixture.close_visit()

    def test_no_ready_never_launches_and_partial_sensor_never_becomes_a_result(self):
        self.fixture.env.update(POSE_TEST='ambiguous')
        self.fixture.stage_visit()
        result = self.fixture.step('case', flip.CASE, check=False)
        self.assertIn(b'refused flip_not_ready', result.stderr)
        self.assertFalse((self.out / 'entered').exists())
        self.assertEqual(set(self.fixture.collect()), {'out/orientation-batch'})
        self.fixture.close_visit()

    def test_mixed_application_log_refuses_both_time_and_image_evidence(self):
        descriptor = self.fixture.root / 'proc/60/fd/2'
        descriptor.unlink()
        descriptor.symlink_to('/dev/null')
        self.fixture.stage_visit()
        result = self.fixture.step('case', flip.CASE, check=False)
        self.assertIn(b'refused early_log_writer', result.stderr)
        self.assertFalse((self.fixture.root / 'scene-called').exists())
        self.assertEqual(set(self.fixture.collect()), {'out/orientation-batch'})
        self.fixture.close_visit()

    def test_rejected_picture_does_not_erase_a_bound_sensor_trace(self):
        self.fixture.env.update(LOSE_TOUCH='1')
        self.fixture.stage_visit()
        self.fixture.step('case', flip.CASE)
        package = self.fixture.collect()
        self.assertEqual(package['out/B-library/shelf'], b'not_observed\n')
        self.assertEqual(len(package['out/B-library/sensor'].splitlines()), flip.SAMPLES)
        self.fixture.close_visit()

    def test_early_bound_sampler_may_finish_before_shell_handles_the_send(self):
        self.fixture.env.update(SPLIT_SEND='expired')
        self.fixture.stage_visit()
        self.fixture.step('case', flip.CASE)
        package = self.fixture.collect()
        self.assertEqual(package['out/B-library/app-binding'], package['out/B-library/sensor-binding'])
        self.assertEqual(len(package['out/B-library/sensor'].splitlines()), flip.SAMPLES)
        self.fixture.close_visit()

    def test_changed_owner_after_early_binding_never_exports_any_case_data(self):
        self.fixture.env.update(SPLIT_SEND='owner')
        self.fixture.stage_visit()
        result = self.fixture.step('case', flip.CASE, check=False)
        self.assertIn(b'refused send_owner', result.stderr)
        self.assertFalse((self.fixture.root / 'scene-called').exists())
        self.assertEqual(set(self.fixture.collect()), {'out/orientation-batch'})
        self.fixture.close_visit()

    def anchor_visit(self, expected, **environment):
        self.fixture.env.update(NATIVE='D:1', **environment)
        self.fixture.stage_visit()
        self.fixture.step('case', flip.CASE)
        package = self.fixture.collect()
        self.assertEqual(package['out/B-library/portrait'], b'D:1\n')
        self.assertEqual(package['out/B-library/portrait-launch'], expected.encode() + b'\n')
        self.assertEqual(len(package['out/B-library/sensor'].splitlines()), flip.SAMPLES)
        self.assertNotIn(b'private-unrecognized-value', b''.join(package.values()))
        self.assertNotIn(b'b' * 32, b''.join(package.values()))
        self.fixture.close_visit()

    def test_current_generation_anchor_can_differ_without_erasing_motion(self):
        self.anchor_visit('U:3', LAUNCH_POSE='U:3')

    def test_a_current_fixed_mode_is_reported_verbatim_not_as_a_holding(self):
        self.anchor_visit('FIXED:1', LAUNCH_POSE='FIXED:1')

    def test_missing_optional_anchor_is_not_a_lost_sensor_trace(self):
        self.anchor_visit('missing', ANCHOR_VARIANT='missing')

    def test_unrecognized_anchor_never_exports_contents(self):
        self.anchor_visit('malformed', ANCHOR_VARIANT='malformed')

    def test_foreign_record_never_exports_direction_or_identity(self):
        self.anchor_visit('foreign_generation', ANCHOR_VARIANT='foreign_generation')

    def test_unsafe_anchor_is_never_followed(self):
        self.anchor_visit('malformed', ANCHOR_VARIANT='unsafe')

    def test_state_open_is_proven_independently_of_optional_anchor(self):
        (self.fixture.lock / 'state-open').write_text('other\n')
        self.fixture.stage_visit()
        result = self.fixture.step('case', flip.CASE, check=False)
        self.assertIn(b'refused send_state_open', result.stderr)
        self.assertFalse((self.fixture.root / 'scene-called').exists())
        self.assertEqual(set(self.fixture.collect()), {'out/orientation-batch'})
        self.fixture.close_visit()

    def test_incomplete_ui_after_checkpoint_exports_only_bound_time_evidence(self):
        self.fixture.env.update(NATIVE='D:1', LAUNCH_POSE='U:3', FINAL_VARIANT='missing')
        self.fixture.stage_visit()
        result = self.fixture.step('case', flip.CASE, check=False)
        self.assertIn(b'refused capture_metadata', result.stderr)
        self.fixture.step('restore')
        package = self.fixture.collect()
        self.assertEqual(set(package), {'out/orientation-batch', *('out/B-library/' + leaf for leaf in flip.BASE_LEAVES)})
        self.assertEqual(package['out/B-library/observation'], b'partial capture_metadata\n')
        self.assertEqual(package['out/B-library/portrait-launch'], b'U:3\n')
        self.assertEqual(len(package['out/B-library/sensor'].splitlines()), flip.SAMPLES)
        self.assertNotIn('out/B-library/shelf', package)
        self.fixture.step('close')
        self.fixture.step('absent')

    def test_spoofed_or_partial_publication_never_qualifies_as_closed_data(self):
        self.fixture.env.update(FINAL_VARIANT='empty')
        self.fixture.stage_visit()
        result = self.fixture.step('case', flip.CASE, check=False)
        self.assertIn(b'refused capture_metadata', result.stderr)
        (self.out / 'observation').write_text('captured startup_turn\n')
        self.assertIn(b'refused public_output', self.fixture.step('collect', check=False).stderr)
        (self.out / 'pose-checkpoint').unlink()
        self.assertEqual(set(self.fixture.collect()), {'out/orientation-batch'})
        self.fixture.close_visit()

    def test_rollover_after_sealed_markers_cannot_publish_pixels_but_keeps_bound_motion(self):
        self.fixture.env.update(ROLL_AFTER_LOG='1')
        self.fixture.stage_visit()
        result = self.fixture.step('case', flip.CASE, check=False)
        self.assertIn(b'refused presentation_owner', result.stderr)
        self.assertFalse((self.fixture.root / 'scene-called').exists())
        self.fixture.step('restore')
        package = self.fixture.collect()
        self.assertEqual(package['out/B-library/observation'], b'partial presentation_owner\n')
        self.assertEqual(set(package), {'out/orientation-batch', *('out/B-library/' + leaf for leaf in flip.BASE_LEAVES)})
        self.fixture.step('close')
        self.fixture.step('absent')


if __name__ == '__main__':
    unittest.main()
