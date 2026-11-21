import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import assisted_sleep_visit as visit


class AssistedSleepVisitTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.label = '0123456789abcdef'
        self.out = self.root / 'output'
        self.out.mkdir()
        self.source = (visit.HERE / 'assisted-sleep-change.sh').read_text()

    def call(self, code, *, expected=0):
        setup = f'''set -eu; out='{self.out}'; root='{self.root}'; label={self.label}; stage='{self.root}'; case_dir='{self.out}';
absent() {{ [ ! -e "$1" ]; }}; plain() {{ [ -f "$1" ]; }}; usleep() {{ :; }}; fail() {{ echo "$*" >&2; exit 77; }}
wake_reason() {{ case "$1" in 26) echo managed_available;; 32) echo task_busy;; 41) echo power_query;; 45) echo sensor_query;; *) echo observer_unknown;; esac; }}
'''
        result = subprocess.run(['sh', '-c', self.source+'\n'+setup+code], capture_output=True, text=True, timeout=7)
        self.assertEqual(result.returncode, expected, result.stderr)
        return result.stdout

    def test_only_fresh_live_phase_with_enough_time_can_prompt(self):
        program = '''
as_hydrate() { requested_phase=$name; }; wake_observe() { [ "$1" = "${GUARD:-sleep-post}" ]; }; as_now() { echo 1000; }
name=wake; : >"$out/sleep-post.row"; echo "${OFFER:-wake 2300}" >"$out/offer"; as_permit
'''
        self.assertEqual(self.call(program), f'permitted {self.label} wake\n')
        for offer in ('sleep 2300','wake 2199','wake 2300 rest','wake unknown'):
            self.assertEqual(self.call(f'OFFER="{offer}"\n'+program,expected=1),'')
        self.assertEqual(self.call('GUARD=post\n'+program,expected=1),'')

    def test_two_spaced_sleeping_observations_precede_wake_and_both_images(self):
        (self.root/'measure').write_text('#!/bin/sh\necho sample-panel\n'); (self.root/'measure').chmod(0o700)
        (self.out/'portrait-launch').write_text('D:1\n')
        (self.root/'counter').write_text('1\n')
        output = self.call('''
as_now() { echo 1000; }; wake_guard() { return 1; }
wake_observe() { instant=$(cat "$root/counter"); instant=$((instant+200000000)); echo "$instant" >"$root/counter";
    case "$1" in pre|sleep-pre) raw=16;; *) raw=15;; esac
    echo "WAKE_OBS $1 $instant $((instant+1000)) $raw 1 none"; }
as_run_cycle; echo "$as_result"
''')
        self.assertEqual(output.splitlines(),[f'PHASE {self.label} {phase}' for phase in ('sleep','turn','wake')]+['observed changed_sleep_cycle'])
        for phase in ('sleep-pre','sleep-post'):
            self.assertEqual(len((self.out/(phase+'.row')).read_text().splitlines()),2)
        self.assertEqual((self.out/'image-mapping').read_text(),'IMAGE pre keep 16 1\nIMAGE post turn 15 1\n')
        self.assertFalse((self.out/'offer').exists()); self.assertEqual((self.out/'initial-diagnostic').read_text(),'INITIAL ready ready 1\n')

    def test_wake_before_turn_or_post_draw_change_cannot_generate_wake_cue_or_image(self):
        (self.root/'counter').write_text('199999999\n')
        output=self.call('''
as_now() { echo 1000; }; as_image(){ as_site=ready; as_reason=ready; as_attempt=1; if [ "$1" = pre ]; then touch "$out/panel.pgm"; else return 1; fi; }
wake_guard(){ [ "$1" = post ]; }
wake_observe(){ [ "$1" = sleep-pre ] || return 1; n=$(cat "$root/counter"); echo $((n+200000000)) >"$root/counter";
  echo "WAKE_OBS sleep-pre $n $((n+10)) 16 1 none"; }
as_run_cycle; echo "$as_result"
''')
        self.assertEqual(output.splitlines(),[f'PHASE {self.label} sleep',f'PHASE {self.label} turn','not_qualified turn_unconfirmed'])
        self.assertFalse((self.out/'resumed.pgm').exists()); self.assertFalse((self.out/'offer').exists())

    def test_failed_owner_after_physical_capture_never_publishes_pixels(self):
        (self.root/'measure').write_text('#!/bin/sh\necho wrong-owner\n'); (self.root/'measure').chmod(0o700)
        self.call('''
wake_observe(){ [ ! -e "$out/panel.pgm.next" ] || return 45; echo 'WAKE_OBS pre 100 200 16 1 D:1'; }
as_run_cycle
''')
        self.assertEqual((self.out/'initial-diagnostic').read_text(),'INITIAL observe_after sensor_query 1\n')
        self.assertFalse((self.out/'panel.pgm').exists()); self.assertFalse((self.out/'panel.pgm.next').exists())

    def test_refusal_receipt_comes_from_the_original_failure_not_a_later_probe(self):
        for status,expected,attempts in [(45,'sensor_query',1),(1,'observer_unknown',1),(26,'managed_available',12),(32,'task_busy',12)]:
            initial=self.out/'initial-diagnostic'
            initial.unlink(missing_ok=True)
            self.call(f'\nas_now() {{ echo 100; }}; wake_observe() {{ return {status}; }}; as_run_cycle\n')
            self.assertEqual(initial.read_text(),f'INITIAL observe_before {expected} {attempts}\n')
            self.assertFalse((self.out/'pre.row').exists()); self.assertFalse((self.out/'offer').exists())

    def test_panel_read_failure_is_reported_separately_from_a_pose_refusal(self):
        (self.root/'measure').write_text('#!/bin/sh\nexit 2\n'); (self.root/'measure').chmod(0o700)
        self.call('''\nwake_observe(){ echo 'WAKE_OBS pre 100 200 16 1 D:1'; }; as_run_cycle\n''')
        self.assertEqual((self.out/'initial-diagnostic').read_text(),'INITIAL image_read raw_panel 1\n')
        self.assertFalse((self.out/'panel.pgm.next').exists()); self.assertFalse((self.out/'offer').exists())

    def test_frozen_script_keeps_normal_restoration_and_has_no_power_state_writer(self):
        script=visit.device_program()
        self.assertLess(len(script.encode('ascii')),32768)
        self.assertNotIn('powerd_test',script); self.assertNotIn('com.lab126.powerd wakeUp',script)
        self.assertIn('restore) restore_trial;;',script); self.assertIn('absent) no_trial',script)
        self.assertIn('permit) as_permit || fail permit;;',script)
        self.assertIn('[ "$as_until" -ge "$((as_check+1200))" ]',script)
        subprocess.run(['sh','-n'],input=script,text=True,check=True)

    def test_capability_query_only_reports_method_flags_never_values_or_raw_debug(self):
        probe=self.root/'lipc-probe'; setter=self.root/'lipc-set-prop'
        setter.write_text('#!/bin/sh\necho changed >&2; exit 1\n'); setter.chmod(0o700)
        probe.write_text('#!/bin/sh\n[ "$#:$1" = 1:com.lab126.powerd ] || exit 3; cat "$LINES"\n'); probe.chmod(0o700)
        sample=self.root/'properties'; env=dict(os.environ,PATH=str(self.root)+':'+os.environ['PATH'],LINES=str(sample))
        for body,wanted,status in [('com.lab126.powerd\n w Int wakeUp\n r Str state\n','write_only_int',0),('com.lab126.powerd:\n r Str state\n','not_listed',0),('com.lab126.powerd\n r Str wakeUp\n','other',0),('w Int wakeUp\nprivate debug data\n','unknown',2),('com.lab126.powerd\n w Int wakeUp\n w Int wakeUp\n','unknown',2)]:
            sample.write_text(body)
            result=subprocess.run(['sh',visit.HERE/'wake_capabilities.sh','powerd-metadata'], capture_output=True,text=True,env=env,timeout=4)
            self.assertEqual((result.returncode,result.stdout,result.stderr),(status,f"WAKE_CAP {'unknown' if status else 'present'} {wanted}\n",''))
