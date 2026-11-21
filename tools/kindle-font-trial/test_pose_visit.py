import io
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import unittest

import pose_visit
from test_orientation_device import archive


class VisitTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.label = 'abcd000000001234'
        self.stage = self.root / 'mnt/us' / ('.weread-orientation-' + self.label)
        self.out = self.stage / 'out/B-reader'
        self.lock = self.root / 'tmp/weread-launcher.lock'
        self.env = dict(os.environ, PATH=str(self.bin)+':'+os.environ['PATH'], FIXTURE=str(self.root))
        self.gen = '1:'+'a'*32

    def program(self, name, text):
        file = self.bin / name
        file.write_text('#!/bin/sh\nset -eu\n'+text+'\n')
        file.chmod(0o755)

    def guard(self, ignore_first=False, changed_owner=False):
        self.out.mkdir(parents=True)
        self.lock.mkdir(parents=True)
        executable = self.stage / 'B/app/weread'
        executable.parent.mkdir(parents=True)
        shutil.copy2(shutil.which('sleep'), executable)
        child = subprocess.Popen([executable, '30'])
        self.addCleanup(lambda: (child.terminate(), child.wait()))
        birth = Path(f'/proc/{child.pid}/stat').read_text().split()[21]
        for key, value in {'generation':self.gen, 'application-pid':child.pid, 'application-birth':birth,
                           'application-owned':f'{self.gen}:{child.pid}:{birth}', 'managed-sleep-request':self.gen+':1',
                           'managed-sleep-ready':self.gen+':1'}.items():
            (self.lock / key).write_text(str(value)+'\n')
        (self.out / 'power-owner').write_bytes((self.lock / 'application-owned').read_bytes())
        state = self.root / 'power'
        state.write_text('active\n')
        self.program('lipc-get-prop', 'cat "$FIXTURE/power"')
        ignore = 'if [ ! -e "$FIXTURE/ignored" ]; then touch "$FIXTURE/ignored"; exit 0; fi' if ignore_first else ':'
        self.program('lipc-set-prop', '[ "$*" = "-i com.lab126.powerd wakeUp 1" ]; echo wake >>"$FIXTURE/calls"; '
                     'if [ "$(cat "$FIXTURE/power")" = screenSaver ]; then '+ignore+'; fi; echo active >"$FIXTURE/power"')
        switch_owner = 'echo replacement >"$FIXTURE/tmp/weread-launcher.lock/application-owned"' if changed_owner else ':'
        self.program('powerd_test', '[ "$*" = -p ]; echo enter >>"$FIXTURE/calls"; echo screenSaver >"$FIXTURE/power"; '+switch_owner)
        helper = self.root / 'alarm'
        subprocess.run(['cc','-Wall','-Wextra','-Werror','-std=c11','-DWEREAD_POSE_VISIT_TEST',
                        pose_visit.HERE / 'pose_visit_alarm.c','-o',helper],check=True)
        result = subprocess.run([helper, 'guarded-cycle', self.out, self.lock], env=self.env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=9, text=True)
        return result, state.read_text().strip(), (self.root / 'calls').read_text().splitlines()

    def test_alarm_retries_successful_wake_that_left_the_same_cycle_asleep(self):
        result, state, calls = self.guard(ignore_first=True)
        self.assertEqual((result.returncode, result.stdout, state), (0,'requested\n','active'))
        self.assertEqual(calls, ['wake','enter','wake','wake'])

    def test_alarm_never_operates_on_a_replacement_owner(self):
        result, state, calls = self.guard(changed_owner=True)
        self.assertEqual((result.returncode, result.stdout, state), (1,'not_qualified power_cycle_unconfirmed\n','screenSaver'))
        self.assertEqual(calls, ['wake','enter'])

    def test_closed_candidate_transaction_does_not_replace_daily_or_publish_private_markers(self):
        volume = self.root / 'mnt/us'; documents = volume/'documents'; documents.mkdir(parents=True)
        for folder in ('tmp','proc','sys/class/input','var/local'):
            (self.root/folder).mkdir(parents=True)
        (self.root/'proc/mounts').write_text(f'local {volume} tmpfs rw 0 0\n')
        daily = documents / 'weread.sh'; daily.write_text('daily\n')
        source = pose_visit.device_program().replace(f'= {pose_visit.ARM_B} ]', '= 6 ]')
        source = re.sub(r'/mnt/us|/sys/class/input|/proc/|/var/local|/tmp(?=[/)])', lambda match: str(self.root)+match[0],source)
        defs, dispatch = source.split('case "$mode" in\n    reference)')
        overrides = '''
native() { return 0; }; state() { cat "$daily"; }; count_doc() { if [ -f "$doc" ]; then echo 1; else echo 0; fi; }
wait_catalog() { [ "$(count_doc)" = "$1" ]; }; scan() { :; }; portrait() { echo U:3; }
'''
        def run(mode, name='none', data=b'', check=True, command=None, mocks=''):
            code = command or 'case "$mode" in\n    reference)'+dispatch
            return subprocess.run(['sh','-c',defs+overrides+mocks+'\n'+code,'fixture',mode,self.label,name],
                                  input=data,capture_output=True,env=self.env,timeout=10,check=check)
        seed = archive([('demo/library.sqlite',b'demo'),('library-covers.json',b'{}')])
        entries = [(member,f'#!/bin/sh\nstage={self.stage}\n'.encode() if member=='document' else
                    seed if member=='seed.tar.gz' else b'public') for member in pose_visit.MEMBERS]
        self.assertEqual(run('admit').stdout,b'admitted\n')
        self.assertEqual(run('stage',data=archive(entries)).stdout,b'indexed\n')
        self.assertNotEqual(run('case','A-reader',check=False).returncode,0)
        self.out.mkdir()
        (self.out/'complete').write_text('B-reader\n')
        for leaf in 'request markers portrait portrait-launch portrait-source portrait-return thumb.pgm panel.pgm final.png final.metadata.json splash splash-status'.split():
            (self.out/leaf).write_text('public')
        (self.out/'power-owner').write_text('secret-identity')
        (self.out/'wake').write_text('not_run alarm_unavailable\n')
        (self.out/'resumed.pgm').write_text('not-qualified')
        with tarfile.open(fileobj=io.BytesIO(run('collect').stdout)) as packed:
            self.assertNotIn('out/B-reader/power-owner',packed.getnames())
            self.assertNotIn('out/B-reader/resumed.pgm',packed.getnames())
        self.assertEqual(run('restore').stdout,b'restored\n')
        self.assertEqual(run('close').stdout,b'closed\n')
        self.assertEqual(run('absent').stdout,b'absent\n')
        self.assertEqual(daily.read_text(),'daily\n')
        self.assertFalse(self.stage.exists())

    def test_frame_capture_refuses_foreign_foreground_and_stopped_gui(self):
        script=(pose_visit.HERE/'pose_visit.sh').read_text().replace('/proc/',str(self.root)+'/proc/')
        manager=self.root/'proc/303'; manager.mkdir(parents=True); (manager/'stat').write_text('303 (awesome) T\n')
        fds=self.root/'proc/999999/fd'; fds.mkdir(parents=True)
        (fds/'1').symlink_to('/dev/fb0'); (fds/'2').symlink_to('/dev/input/event1')
        for overlay,task,expected in [(False,'S',True),(True,'S',False),(False,'T',False)]:
            masks='true' if overlay else 'false'
            harness = f'''
power_state() {{ echo active; }}; same_gui() {{ :; }}; task_state() {{ echo {task}; }}
absent() {{ if {masks}; then return 1; else return 0; fi; }}; pidof() {{ [ "$1" = awesome ] && echo 303; }}
gui=999999; public_foreground
'''
            result=subprocess.run(['sh','-c',script+'\n'+harness],capture_output=True,timeout=4)
            self.assertEqual(result.returncode == 0, expected)

    def test_wake_cycle_requires_actual_state_same_owner_and_cycle_records(self):
        self.out.mkdir(parents=True); self.lock.mkdir(parents=True)
        generation=self.gen; identity='100:42'; state=self.root/'state'
        for name,value in [('generation',generation),('managed-sleep-available',generation),('application-owned',generation+':'+identity)]:
            (self.lock/name).write_text(value+'\n')
        self.program('powerd_test', ':'); self.program('lipc-set-prop', ':')
        wake=self.stage/'pose-wake'; wake.write_text(f'''#!/bin/sh
echo screenSaver >'{state}'
for m in managed-sleep-request managed-sleep-ready; do echo '{generation}:1' >"$3/$m"; done
sleep 0.2
echo active >'{state}'
for m in managed-wake-request managed-wake-ready; do echo '{generation}:1' >"$3/$m"; done
echo '{generation}:1:D:1' >"$3/managed-native-portrait-calibration"
echo requested
'''); wake.chmod(0o755)
        meter=self.stage/'measure'; meter.write_text('#!/bin/sh\necho public-reader\n'); meter.chmod(0o755)
        state.write_text('active\n')
        harness=f'''
stage='{self.stage}'; out='{self.out}'; lock='{self.lock}'; generation='{generation}'; identity='{identity}'
same_gui() {{ :; }}; public_foreground() {{ [ "$(power_state)" = active ]; }}; plain() {{ [ -f "$1" ]; }}; absent() {{ [ ! -e "$1" ]; }}
power_state() {{ cat '{state}'; }}; task_state() {{ if [ "$(power_state)" = active ]; then echo S; else echo T; fi; }}
usleep() {{ sleep 0.03; }}; fail() {{ exit 1; }}; pose_wake
'''
        run=subprocess.run(['sh','-c',(pose_visit.HERE/'pose_visit.sh').read_text()+harness],capture_output=True,env=self.env,timeout=8)
        self.assertEqual((run.returncode,run.stdout),(0,b'observed screenSaver stopped same_process wake_ready active native D:1\n'),run.stderr)
        self.assertEqual((self.out/'resumed.pgm').read_text(),'public-reader\n')

    def test_visit_preserves_measured_splash_reversal_and_drops_unowned_scanout(self):
        self.out.mkdir(parents=True); self.lock.mkdir(parents=True)
        home=self.stage/'B'; log=home/'data/logs'; log.mkdir(parents=True)
        (log/'application-bootstrap.log').write_text('WEREAD_STARTUP 22.100000000 first_application_frame_send\n')
        (self.lock/'generation').write_text(self.gen+'\n')
        (self.out/'entered').mkdir(); (self.out/'entered/arm').write_text('B-reader\n'); (self.out/'status').write_text('0\n')
        for name, body in [('measure', 'printf "public-%s\\n" "$1"'),
                           ('pose-splash', 'echo "SPLASH 100 200 1 32 0"; echo "splash_refused rotated" >&2; exit 20')]:
            path=self.stage/name; path.write_text('#!/bin/sh\n'+body+'\n'); path.chmod(0o755)
        harness=f'''
stage='{self.stage}'; out='{self.out}'; lock='{self.lock}'; home='{home}'; name=B-reader; label={self.label}; gui=155; birth=41
plain() {{ [ -f "$1" ]; }}; absent() {{ [ ! -e "$1" ]; }}; prepare_case() {{ :; }}; same_gui() {{ :; }}; gui_args() {{ :; }}; live_pose() {{ :; }}
pose_wake() {{ echo 'not_run alarm_unavailable'; }}; pose_recover_wake() {{ :; }}; finish_case() {{ :; }}; sleep() {{ :; }}; usleep() {{ :; }}
fail() {{ exit 1; }}; observations=0; public_foreground() {{ observations=$((observations+1)); [ "$observations" != "${{LOSE_AT:-0}}" ]; }}
set -eu; pose_case
'''
        def replay(lose_at):
            return subprocess.run(['sh','-c',(pose_visit.HERE/'pose_visit.sh').read_text()+harness],capture_output=True,
                                  env=self.env|{'LOSE_AT':lose_at},timeout=5)
        result=replay('0'); self.assertEqual((result.returncode,result.stdout),(0,b'completed B-reader\n'),result.stderr)
        self.assertEqual(((self.out/'splash').read_text(),(self.out/'splash-status').read_text()),('SPLASH 100 200 1 32 0\n','20\n'))
        self.assertEqual((self.out/'wake').read_text(),'not_run alarm_unavailable\n')
        self.assertNotEqual(replay('2').returncode,0)
        self.assertFalse((self.out/'panel.pgm').exists()); self.assertFalse((self.out/'thumb.pgm').exists())
