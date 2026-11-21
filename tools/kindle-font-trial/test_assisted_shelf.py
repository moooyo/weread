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
        (self.lock / 'generation').write_text('private-generation\n')
        commands = self.root / 'bin'
        commands.mkdir()
        timer = commands / 'lipc-set-prop'
        timer.write_text('#!/bin/sh\ntest "$*" = "-i com.lab126.powerd touchScreenSaverTimeout 1"\n')
        timer.chmod(0o755)
        self.env = dict(os.environ, NATIVE='U:3', RAW='15', TARGET='settings', ROOT=str(self.root),
                        PATH=str(commands) + ':' + os.environ['PATH'])
        source = visit.device_program().replace(f'= {visit.ARM_B} ]', '= 6 ]')
        source = re.sub(r'/mnt/us|/sys/class/input|/proc/|/var/local|/tmp(?=[/)])', lambda match: str(self.root) + match[0], source)
        boundary = 'case "$mode" in\n    reference)'
        self.definitions, dispatch = source.split(boundary)
        self.dispatch = boundary + dispatch
        self.mocks = '''
native() { :; }; state() { cat "$daily"; }; count_doc() { if [ -f "$doc" ]; then echo 1; else echo 0; fi; }
wait_catalog() { [ "$(count_doc)" = "$1" ]; }; scan() { :; }; portrait() { echo "$NATIVE"; }
sleep() { :; }; usleep() { :; }; gui_args() { gui=60; birth=55; :; }
live_pose() { cp "$out/portrait" "$out/portrait-launch"; }; public_foreground() { test "${DENY:-0}" = 0; }
'''
        measure = f'''#!/bin/sh
set -eu
test "$1:$2" = request:{LABEL}
stage='{self.stage}'; out=$stage/out/B-library; test "$(cat "$stage/armed")" = B-library
mkdir "$out/entered"; mv "$stage/armed" "$out/entered/arm"
mkdir -p "$stage/B/data/logs"; echo 'WEREAD_STARTUP 123.000000000 first_application_frame_send' >"$stage/B/data/logs/application-bootstrap.log"
echo 'REQUEST 122.000000000'; echo private-pixels >"$out/final.png"
printf '{{"route":"%s"}}' "$TARGET" >"$out/final.metadata.json"; echo 0 >"$out/status"
'''
        entries = {'document': f'#!/bin/sh\nstage={self.stage}\n'.encode(), 'measure': measure.encode(),
                   'seed.tar.gz': archive([('library-covers.json', b'{}'), ('demo/library.sqlite', b'SQLite')]),
                   'pose-snapshot': b'#!/bin/sh\n[ "$1" = 10 ] || exit 1; for n in $(seq 1 10); do echo "$n ${n}00 ${n}01 $RAW"; done\n',
                   'shelf-scene': b'#!/bin/sh\necho "SHELF 124000000000 124001000000 3 grid"\n'}
        self.payload = archive([(key, entries.get(key, b'public')) for key in visit.MEMBERS])

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
        self.assertNotIn('private-generation', repr(results))
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


if __name__ == '__main__':
    unittest.main()
