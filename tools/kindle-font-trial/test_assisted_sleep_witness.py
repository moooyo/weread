from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import assisted_sleep_visit as visit


class WitnessTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name); self.out=self.root/'out'; self.out.mkdir()
        self.label='1234567890abcdef'; self.script=(visit.HERE/'assisted-sleep-change.sh').read_text()+(visit.HERE/'assisted-sleep-witness.sh').read_text()

    def call(self, code, *, expected=0):
        helpers=f'''
set -eu
stage='{self.root}'; out='{self.out}'; label={self.label}; clock="$out/test-clock"; echo 10000 >"$clock"
as_now() {{ cat "$clock"; }}; usleep() {{ now=$(as_now); echo "$((now+100))" >"$clock"; }};
fail() {{ exit 66; }}; plain() {{ [ -f "$1" ]; }}; absent() {{ [ ! -e "$1" ]; }}
'''
        result=subprocess.run(['sh','-c',self.script+'\n'+helpers+code],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,expected,result.stderr)
        return result.stdout

    def test_window_keeps_unchanged_and_opposite_directions_from_the_same_sleep(self):
        for change in (False,True):
            trace=self.out/'sleep-trace'; trace.unlink(missing_ok=True)
            self.call('''
wake_guard(){ [ "$1" = sleep-post ]; }; wake_observe_any(){ now=$(as_now); raw=16; '''+
                ('[ "$now" -lt 11200 ] || raw=15;' if change else ':;')+'''
                [ "$now" != 10500 ] || return 1; echo "WAKE_OBS sleep-post $((now*10000000)) $((now*10000000+1000)) $raw 1 none"; }
witness_window
''')
            first,last,attempts,skipped,ending=(self.out/'turn-window').read_text().split()[1:]
            self.assertEqual((int(last)-int(first),int(attempts),int(skipped),ending),(2200,22,1,'elapsed'))
            rows=trace.read_text().splitlines(); self.assertEqual(len(rows),21)
            self.assertEqual({row.split()[4] for row in rows},{'16','15'} if change else {'16'})

    def test_early_wake_keeps_seen_rows_and_never_offers_a_power_action(self):
        result=self.call('''
wake_observe_any(){ [ "$(as_now)" -lt 10400 ] || return 1; echo "WAKE_OBS sleep-post 11000 12000 16 1 none"; }
wake_guard(){ [ "$1" = post ] && [ "$(as_now)" -ge 10400 ]; }
witness_window
''')
        self.assertEqual(result, f'PHASE {self.label} turn\n'); self.assertFalse((self.out/'offer').exists())
        self.assertTrue((self.out/'turn-window').read_text().rstrip().endswith(' early_wake'))

    def test_elapsed_stable_same_direction_can_receive_normal_conditional_wake(self):
        result=self.call('''
as_hydrate(){ requested_phase=$name; }; wake_guard(){ [ "$1" = sleep-post ]; }; wake_observe_any(){ return 47; }
name=wake; echo 'wake 12000' >"$out/offer"; echo 'WINDOW 4000 6200 21 1 elapsed' >"$out/turn-window"; witness_permit
''')
        self.assertEqual(result,f'permitted {self.label} wake\n')

    def test_no_wake_permit_if_screen_is_active_or_window_cut_short(self):
        for end,guard in [('elapsed','false'),('early_wake','true'),('lost_state','true')]:
            self.call(f'''
as_hydrate(){{ requested_phase=$name; }}; wake_guard(){{ {guard}; }}; name=wake; echo 'wake 12000' >"$out/offer";
echo 'WINDOW 4000 6200 21 0 {end}' >"$out/turn-window"; witness_permit
''',expected=1)

    def test_final_shutdown_only_targets_the_live_bound_foreground(self):
        self.call('''
wake_guard(){ return 1; }; kill(){ touch "$out/signal"; }; witness_finish
''')
        self.assertFalse((self.out/'signal').exists())

    def test_guarded_finish_rejects_other_birth_and_generation_then_signals_only_the_owned_child(self):
        source=(visit.HERE/'assisted_sleep_finish.c').read_text().replace('"/mnt/us/.weread-orientation-%s"',f'"{self.root}/s-%s"').replace('"/tmp/weread-launcher.lock"',f'"{self.root}/lock"')
        code=self.root/'finish.c'; code.write_text(source); helper=self.root/'finish'
        subprocess.run(['cc','-Wall','-Wextra','-Werror','-std=c11',code,'-o',helper],check=True)
        stage=self.root/('s-'+self.label); gui=stage/'B/app/weread'; gui.parent.mkdir(parents=True); shutil.copy2(shutil.which('sleep'),gui)
        out=stage/'out/B-reader'; entered=out/'entered'; entered.mkdir(parents=True); lock=self.root/'lock'; lock.mkdir()
        child=subprocess.Popen([gui,'15']); self.addCleanup(lambda: (child.terminate(),child.wait()))
        birth=Path(f'/proc/{child.pid}/stat').read_text().split()[21]; generation='1:'+'b'*32
        expected=f'{generation}:{child.pid}:{birth}\n'; (stage/'owner').write_text(self.label+'\n'); (entered/'arm').write_text('B-reader\n'); (out/'wake-owner').write_text(expected)
        (lock/'generation').write_text(generation+'\n'); (lock/'application-owned').write_text(expected)
        (lock/'managed-wake-ready').write_text(generation+':1\n'); (lock/'managed-wake-request').write_text(generation+':2\n')
        def close(value): return subprocess.run([helper,self.label,str(child.pid),value],timeout=2).returncode
        self.assertEqual(close(birth),2); self.assertIsNone(child.poll()); (lock/'managed-wake-request').write_text(generation+':1\n')
        self.assertEqual(close(str(int(birth)+1)),2); self.assertIsNone(child.poll()); self.assertEqual(close(birth),0); self.assertEqual(child.wait(timeout=2),-15)

    def test_real_post_sensor_is_retained_even_when_no_picture_can_be_published(self):
        self.call('''
wake_guard(){ [ "$1" = post ]; }; wake_observe_any(){ echo 'WAKE_OBS post 998 1005 16 1 U:3'; }
as_image(){ as_site=image_read; return 1; }; witness_post
''')
        self.assertEqual((self.out/'post-status.row').read_text(),'WAKE_OBS post 998 1005 16 1 U:3\n')
        self.assertFalse((self.out/'resumed.pgm').exists())

    def test_only_new_visit_allows_unselected_postures_and_joint_exit_receipt(self):
        baseline=visit.device_program(); witness=visit.device_program(witness=True)
        self.assertNotIn('wake_observe_any',baseline); self.assertNotIn('witness_finish',baseline)
        self.assertIn('as_row=$(wake_observe_any "$1")',witness); self.assertIn('sleep-trace turn-window',witness)
        self.assertIn('observed resident_wake',witness); self.assertNotIn('observed changed_sleep_cycle',witness)
        self.assertLessEqual(len(witness.encode('ascii')),32768)
        subprocess.run(['sh','-n'],input=witness,text=True,check=True)
