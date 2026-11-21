import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import unittest

import assisted_shelf as visit
from test_orientation_device import archive


LABEL = '0123456789abcdef'


class AssistedDeviceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        volume = self.root / 'mnt/us'
        docs = volume / 'documents'
        docs.mkdir(parents=True)
        for folder in ('tmp', 'var/local', 'proc', 'sys/class/input'):
            (self.root / folder).mkdir(parents=True)
        (self.root / 'proc/mounts').write_text(f'local {volume} tmpfs rw 0 0\n')
        self.daily = docs / 'weread.sh'
        self.daily.write_text('daily\n')
        self.stage = volume / ('.weread-orientation-' + LABEL)
        self.out = self.stage / 'out/B-library'
        self.lock = self.root / 'tmp/weread-launcher.lock'
        self.lock.mkdir()
        (self.lock / 'nonce').write_text('a' * 32 + '\n')
        (self.lock / 'generation').write_text('1:' + 'a' * 32 + '\n')
        (self.lock / 'state-open').write_bytes((self.lock / 'generation').read_bytes())
        (self.lock / 'application-owned').write_text('1:' + 'a' * 32 + ':60:55\n')
        (self.lock / 'application-pid').write_text('60\n')
        (self.lock / 'application-birth').write_text('55\n')
        gui_proc = self.root / 'proc/60'
        (gui_proc / 'fd').mkdir(parents=True)
        (gui_proc / 'stat').write_text(' '.join(['60', '(weread)', 'S', *(['0'] * 18), '55']) + '\n')
        (gui_proc / 'fd/6').symlink_to('/dev/fb0')
        (gui_proc / 'fd/7').symlink_to('/dev/input/event1')
        manager = self.root / 'proc/70'
        manager.mkdir()
        (manager / 'stat').write_text(' '.join(['70', '(awesome)', 'T', *(['0'] * 18), '81']) + '\n')
        commands = self.root / 'bin'
        commands.mkdir()
        timer = commands / 'lipc-set-prop'
        timer.write_text('#!/bin/sh\ntest "$*" = "-i com.lab126.powerd touchScreenSaverTimeout 1"\n')
        timer.chmod(0o755)
        self.env = dict(os.environ, NATIVE='U:3', RAW='15', TARGET='settings', ROOT=str(self.root),
                        PATH=str(commands) + ':' + os.environ['PATH'])
        self.select_plan()
        self.mocks = '''
native() { :; }; state() { cat "$daily"; }; count_doc() { if [ -f "$doc" ]; then echo 1; else echo 0; fi; }
wait_catalog() { [ "$(count_doc)" = "$1" ]; }; scan() { :; }; portrait() {
    if [ -e "$ROOT/samples-read" ]; then echo "${NATIVE_AFTER:-$NATIVE}"; else echo "$NATIVE"; fi
}
sleep() { :; }; usleep() { :; }; gui_owned() { gui=60; birth=55; :; }; gui_args() { gui_owned; }
power_state() { echo "${POWER:-active}"; }; pidof() { [ "$1" = awesome ] && echo 70; }
public_foreground() { test "${DENY:-0}" = 0; }
'''
        measure = f'''#!/bin/sh
set -eu
test "$1:$2" = request:{LABEL}
stage='{self.stage}'; out=$stage/out/B-library; test "$(cat "$stage/armed")" = B-library
mkdir "$out/entered"; mv "$stage/armed" "$out/entered/arm"
printf '%s:%s\\n' "$(cat "$ROOT/tmp/weread-launcher.lock/generation")" "${{LAUNCH_POSE:-$NATIVE}}" >"$ROOT/tmp/weread-launcher.lock/native-portrait-calibration"
mkdir -p "$stage/B/data/logs"; echo 'WEREAD_STARTUP 123.000000000 first_application_frame_send' >"$stage/B/data/logs/application-bootstrap.log"
echo 'REQUEST 122.000000000'; echo private-pixels >"$out/final.png"
printf '{{"route":"%s"}}' "$TARGET" >"$out/final.metadata.json"; echo 0 >"$out/status"
'''
        entries = {'document': f'#!/bin/sh\nstage={self.stage}\n'.encode(), 'measure': measure.encode(),
                   'seed.tar.gz': archive([('library-covers.json', b'{}'), ('demo/library.sqlite', b'SQLite')]),
                   'pose-snapshot': b'#!/bin/sh\n[ "$1" = 10 ] || exit 1; touch "$ROOT/samples-read"; for n in $(seq 1 "${COUNT:-10}"); do if [ "$n" = 2 ]; then raw=${SECOND_RAW:-$RAW}; else raw=$RAW; fi; echo "$n ${n}00 ${n}01 $raw"; done\n',
                   'shelf-scene': b'#!/bin/sh\ntouch "$ROOT/scene-called"; if [ "${LOSE_TOUCH:-0}" = 1 ]; then rm "$ROOT/proc/60/fd/7"; fi; echo "SHELF 124000000000 124001000000 ${NATIVE#*:} grid"\n'}
        self.payload = archive([(key, entries.get(key, b'public')) for key in visit.MEMBERS])

    def select_plan(self, fixed_down=False, bounded_up=False):
        source = visit.device_program(fixed_down=fixed_down, bounded_up=bounded_up).replace(f'= {visit.ARM_B} ]', '= 6 ]')
        source = re.sub(r'/mnt/us|/sys/class/input|/proc/|/var/local|/tmp(?=[/)])', lambda match: str(self.root) + match[0], source)
        boundary = 'case "$mode" in\n    reference)'
        self.definitions, dispatch = source.split(boundary)
        self.dispatch = boundary + dispatch

    def step(self, mode, name='none', data=b'', check=True):
        result = subprocess.run(['sh', '-ec', self.definitions + self.mocks + self.dispatch, 'assisted-fixture', mode, LABEL, name],
                                input=data, capture_output=True, env=self.env, timeout=8)
        if check:
            self.assertEqual(result.returncode, 0, f'{mode}: {result.stderr!r}')
        return result

    def stage_visit(self):
        self.assertEqual(self.step('admit').stdout, b'admitted\n')
        self.assertEqual(self.step('reference').stdout.decode().strip(), self.env['NATIVE'])
        self.assertEqual(self.step('stage', data=self.payload).stdout, b'indexed\n')

    def collect(self):
        with tarfile.open(fileobj=io.BytesIO(self.step('collect').stdout)) as packed:
            return {member.name: packed.extractfile(member).read() for member in packed}

    def close_visit(self):
        self.assertEqual(self.step('restore').stdout, b'restored\n')
        self.assertEqual(self.step('close').stdout, b'closed\n')
        self.assertEqual(self.step('absent').stdout, b'absent\n')
        self.assertFalse(self.stage.exists())
        self.assertEqual(self.daily.read_text(), 'daily\n')

    def test_scoped_library_visit_preserves_true_route_and_no_images(self):
        self.stage_visit()
        self.assertNotEqual(self.step('case', 'B-reader', check=False).returncode, 0)
        self.assertEqual(self.step('case', 'B-library').stdout, b'completed B-library\n')
        self.assertFalse((self.out / 'final.png').exists())
        self.assertEqual((self.out / 'portrait-launch').read_text(), 'U:3\n')
        self.assertEqual((self.out / 'observation').read_text(), 'captured candidate_library_exit\n')
        results = self.collect()
        self.assertEqual(json.loads(results['out/B-library/final.metadata.json'])['route'], 'settings')
        self.assertEqual(results['out/B-library/shelf'], b'SHELF 124000000000 124001000000 3 grid\n')
        self.assertNotIn('a' * 32, repr(results))
        self.assertFalse(any(name.endswith(('.pgm', '.png')) for name in results))
        self.close_visit()

    def test_actual_missed_route_is_not_rewritten(self):
        self.env['TARGET'] = 'search'
        self.stage_visit()
        self.step('case', 'B-library')
        self.assertEqual(json.loads(self.collect()['out/B-library/final.metadata.json'])['route'], 'search')
        self.close_visit()

    def test_original_native_pose_never_arms_and_still_closes(self):
        self.env.update(NATIVE='D:1', RAW='16')
        self.check_gate()

    def test_sensor_disagreement_never_arms_and_still_closes(self):
        self.env['RAW'] = '16'
        self.check_gate()

    def check_gate(self):
        self.stage_visit()
        self.step('case', 'B-library')
        results = self.collect()
        self.assertEqual(results['out/B-library/observation'], b'not_started pose_gate\n')
        self.assertNotIn('out/B-library/request', results)
        self.assertFalse((self.out / 'entered').exists())
        self.assertFalse((self.stage / 'armed').exists())
        self.close_visit()

    def test_foreign_foreground_results_are_not_archived(self):
        self.stage_visit()
        self.env['DENY'] = '1'
        self.assertNotEqual(self.step('case', 'B-library', check=False).returncode, 0)
        self.assertEqual(set(self.collect()), {'out/orientation-batch'})
        self.close_visit()

    def test_explicit_down_plan_accepts_both_down_pressure_codes(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='16', SECOND_RAW='18')
        self.stage_visit()
        self.assertEqual(self.step('case', 'B-library').stdout, b'completed B-library\n')
        results = self.collect()
        self.assertEqual(results['out/B-library/observation'], b'captured candidate_library_exit\n')
        self.assertEqual(results['out/B-library/portrait-launch'], b'D:1\n')
        self.assertEqual(results['out/B-library/shelf'], b'SHELF 124000000000 124001000000 1 grid\n')
        self.assertEqual(json.loads(results['out/B-library/final.metadata.json'])['route'], 'settings')
        self.close_visit()

    def test_down_plan_rejects_upright_before_launch(self):
        self.select_plan(fixed_down=True)
        self.check_gate()

    def test_down_plan_rejects_one_conflicting_sensor_reading(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='16', SECOND_RAW='17')
        self.check_gate()

    def test_down_plan_requires_second_matching_native_read(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='16', NATIVE_AFTER='U:3')
        self.check_gate()

    def test_down_plan_requires_ten_readings_and_no_partial_report(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='16', COUNT='9')
        self.stage_visit()
        result = self.step('case', 'B-library', check=False)
        self.assertEqual((result.returncode, result.stderr), (1, b'refused sensor_format\n'))
        self.assertFalse((self.out / 'entered').exists())
        self.assertEqual(set(self.collect()), {'out/orientation-batch'})
        self.close_visit()

    def test_down_launch_anchor_cannot_belong_to_the_other_pose(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='18', LAUNCH_POSE='U:3')
        self.stage_visit()
        result = self.step('case', 'B-library', check=False)
        self.assertEqual((result.returncode, result.stderr), (1, b'refused gui_identity_or_pose\n'))
        self.assertEqual(set(self.collect()), {'out/orientation-batch'})
        self.close_visit()

    def test_down_pre_guard_refuses_before_scene_helper_runs(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='16', POWER='screenSaver')
        self.stage_visit()
        result = self.step('case', 'B-library', check=False)
        self.assertEqual((result.returncode, result.stderr), (1, b'refused pre_power\n'))
        self.assertFalse((self.root / 'scene-called').exists())
        self.assertEqual(set(self.collect()), {'out/orientation-batch'})
        self.close_visit()

    def test_down_post_guard_discards_pending_scene_after_touch_fd_retires(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='16', LOSE_TOUCH='1')
        self.stage_visit()
        result = self.step('case', 'B-library', check=False)
        self.assertEqual((result.returncode, result.stderr), (1, b'refused post_touch\n'))
        self.assertTrue((self.root / 'scene-called').is_file())
        self.assertFalse((self.out / 'shelf.next').exists())
        self.assertFalse((self.out / 'shelf').exists())
        self.assertEqual(set(self.collect()), {'out/orientation-batch'})
        self.close_visit()

    def test_down_publish_failure_is_distinct_and_does_not_archive(self):
        self.select_plan(fixed_down=True)
        self.env.update(NATIVE='D:1', RAW='16')
        self.stage_visit()
        self.mocks += 'mv() { [ "${1##*/}" != shelf.next ] || return 1; command mv "$@"; }\n'
        result = self.step('case', 'B-library', check=False)
        self.assertEqual((result.returncode, result.stderr), (1, b'refused scene_publish\n'))
        self.assertTrue((self.root / 'scene-called').is_file())
        self.assertFalse((self.out / 'shelf.next').exists())
        self.assertFalse((self.out / 'shelf').exists())
        self.assertEqual(set(self.collect()), {'out/orientation-batch'})
        self.close_visit()

    def test_down_contract_cannot_relabel_an_upright_device_program(self):
        plan = visit.DOWN_CONTRACT | {'label': LABEL}
        (self.root / 'plan.json').write_text(json.dumps(plan))
        (self.root / 'device.sh').write_text(visit.device_program())
        with self.assertRaisesRegex(ValueError, 'Incorrect frozen device program'):
            visit.verify_recovery(self.root)
        self.assertIs(visit.contract_for(plan), visit.DOWN_CONTRACT)
        for mixed in (plan | {'expected_calibration': 'U:3'}, plan | {'purpose': visit.CONTRACT['purpose']}):
            with self.assertRaisesRegex(ValueError, 'contract'):
                visit.contract_for(mixed)

    def test_explicit_bounded_up_qualifies_both_up_pressures_and_the_same_launch(self):
        self.select_plan(bounded_up=True)
        self.env['SECOND_RAW'] = '17'
        self.stage_visit()
        self.step('case', 'B-library')
        results = self.collect()
        self.assertEqual(results['out/B-library/portrait-launch'], b'U:3\n')
        self.assertEqual(results['out/B-library/shelf'], b'SHELF 124000000000 124001000000 3 grid\n')
        self.assertEqual(json.loads(results['out/B-library/final.metadata.json'])['route'], 'settings')
        self.close_visit()

    def test_bounded_up_still_refuses_one_down_read_before_launch(self):
        self.select_plan(bounded_up=True)
        self.env['SECOND_RAW'] = '16'
        self.check_gate()

    def test_bounded_up_discards_a_read_when_foreground_is_lost(self):
        self.select_plan(bounded_up=True)
        self.env['LOSE_TOUCH'] = '1'
        self.stage_visit()
        result = self.step('case', 'B-library', check=False)
        self.assertEqual((result.returncode, result.stderr), (1, b'refused post_touch\n'))
        self.assertFalse((self.out / 'shelf.next').exists())
        self.assertEqual(set(self.collect()), {'out/orientation-batch'})
        self.close_visit()

    def test_bounded_up_metadata_and_script_must_match_without_relabelling_either_pose(self):
        original = visit.CONTRACT | {'label': LABEL}
        marked = original | {'foreground_guard': visit.BOUNDED_UP_GUARD}
        for plan, script, ok in [(original, visit.device_program(), True), (marked, visit.device_program(bounded_up=True), True),
                                 (marked, visit.device_program(), False), (original, visit.device_program(bounded_up=True), False),
                                 (marked | visit.DOWN_CONTRACT, visit.device_program(fixed_down=True), False),
                                 (marked | {'foreground_guard': 'unknown'}, visit.device_program(bounded_up=True), False)]:
            (self.root / 'plan.json').write_text(json.dumps(plan))
            (self.root / 'device.sh').write_text(script)
            if ok:
                self.assertEqual(visit.verify_recovery(self.root), plan)
            else:
                with self.assertRaises(ValueError):
                    visit.verify_recovery(self.root)
        with self.assertRaises(ValueError):
            visit.device_program(fixed_down=True, bounded_up=True)


if __name__ == '__main__':
    unittest.main()
