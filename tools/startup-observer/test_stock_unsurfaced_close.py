import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

from test_daily_render_device import HERE, LABEL, MAIN, RELEASE


class UnenteredStockCloseTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.stage = Path(directory.name).resolve()
        self.shell = ['busybox', 'sh'] if os.getenv('WEREAD_TEST_BUSYBOX') else ['sh']
        daily = (HERE / 'daily_render_device.sh').read_text()
        self.close = daily.split(MAIN)[0].replace('stage=/mnt/us/.weread-render-$label', 'stage=$STOCK_FIXTURE')
        self.close += '''
single(){ plain "$1" && [ "$(stat -c %h "$1")" = 1 ]; }
native_idle(){ absent "$stage/other-process" && absent "$stage/unidentified-lock"; }
state(){ native_idle && absent "$stage/native-drift"; }
stock_request_absent(){ absent "$stage/pending-request"; }
channel_idle(){ absent "$stage/input-busy"; }
'''
        self.close += (HERE / 'stock-unsurfaced-close.sh').read_text() + '\n"$mode"\n'
        self.entry = '''set -eu
stage=$STOCK_FIXTURE; label=$1
regular(){ [ -f "$1" ] && [ ! -L "$1" ]; }; missing(){ [ ! -e "$1" ] && [ ! -L "$1" ]; }
fixed(){ regular "$stage/$1" && printf '%s\\n' "$2" | cmp -s - "$stage/$1"; }
'''
        self.entry += (HERE / 'stock-selector-gate.sh').read_text() + '''
stock_selector_gate || exit 1
case "$2" in
    refuse) exit 1;;
    hold) touch "$stage/shell-in-gate"; sleep 3; exit 1;;
    exec) (set -C; printf '%s\\n' "$label" >"$stage/selector-entered"); stock_selector_commit || exit 1; exec true;;
esac
'''
        for name in ('owner','selector-live','tile-dispatch-intent'):
            (self.stage / name).write_text(LABEL+'\n')
        (self.stage / 'armed').write_text(LABEL+':1\n')
        for name in ('tile-release','stock-confirmed-release'):
            (self.stage / name).write_text(RELEASE+'\n')

    def run_shell(self, script, *args, background=False):
        argv = [*self.shell, '-c', script, 'fixture', *args]
        options = dict(env=os.environ | {'STOCK_FIXTURE':str(self.stage)}, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if background:
            child = subprocess.Popen(argv, **options)
            def end():
                if child.poll() is None:
                    child.terminate()
                child.communicate(timeout=7)
            self.addCleanup(end)
            return child
        return subprocess.run(argv, timeout=12, **options)

    def quarantine(self, *args):
        return self.run_shell(self.close, 'stock_quarantine_no_group', LABEL, *args)

    def test_positive_decline_is_retired_and_requires_an_independent_settled_check(self):
        self.assertNotEqual(self.run_shell(self.entry, LABEL, 'refuse').returncode, 0)
        callback = (self.stage/'selector-callback').read_text()
        self.assertEqual((self.stage/'selector-declined').read_text(), callback)
        result = self.quarantine()
        self.assertEqual((result.returncode,result.stdout,result.stderr), (0,b'',b''))
        self.assertEqual((self.stage/'stopped').read_text(), LABEL+'\n')
        self.assertFalse((self.stage/'group').exists())
        self.assertEqual(self.run_shell(self.close,'stock_quarantine_settled',LABEL).returncode, 0)
        (self.stage/'other-process').touch()
        self.assertNotEqual(self.run_shell(self.close,'stock_quarantine_settled',LABEL).returncode, 0)

    def test_late_callback_sees_disarm_before_it_can_enter(self):
        child = self.run_shell(self.close, 'stock_quarantine_no_group', LABEL, background=True)
        limit = time.monotonic()+3
        while not (self.stage/'selector-disarmed').exists() and time.monotonic()<limit:
            time.sleep(.02)
        self.assertTrue((self.stage/'selector-disarmed').exists())
        self.assertNotEqual(self.run_shell(self.entry, LABEL, 'exec').returncode, 0)
        self.assertEqual(child.communicate(timeout=9), (b'',b''))
        self.assertEqual(child.returncode, 0)
        self.assertFalse((self.stage/'selector-entered').exists())

    def test_missing_receipts_active_ordinary_or_extra_callback_never_certify(self):
        self.run_shell(self.entry,LABEL,'refuse')
        for name in ('other-process','unidentified-lock','native-drift','pending-request','input-busy','group','session','selector-entered','selector-duplicate'):
            with self.subTest(name=name):
                marker=self.stage/name; marker.touch()
                self.assertNotEqual(self.quarantine().returncode,0)
                self.assertFalse((self.stage/'stopped').exists())
                marker.unlink()
        (self.stage/'stock-confirmed-release').write_text('different\n')
        self.assertNotEqual(self.quarantine().returncode,0)

    def test_no_callback_is_not_proved_by_idle_or_lapsed_time(self):
        self.assertNotEqual(self.quarantine().returncode, 0)
        self.assertFalse((self.stage/'stopped').exists())
        self.assertFalse((self.stage/'selector-declined').exists())

    def test_matching_labeled_pid_that_is_still_alive_cannot_close(self):
        child = subprocess.Popen(['sleep', '30'])
        self.addCleanup(child.wait)
        self.addCleanup(child.terminate)
        birth = Path(f'/proc/{child.pid}/stat').read_text().rsplit(') ',1)[1].split()[19]
        stamp = f'{LABEL}:{child.pid}:{birth}\n'
        for name in ('selector-callback','selector-declined'):
            (self.stage/name).write_text(stamp)
        self.assertNotEqual(self.quarantine().returncode, 0)
        self.assertIsNone(child.poll())
        self.assertFalse((self.stage/'stopped').exists())

    def test_entered_or_second_callback_never_authorizes_quarantine(self):
        self.assertEqual(self.run_shell(self.entry,LABEL,'exec').returncode,0)
        self.assertFalse((self.stage/'selector-declined').exists())
        self.assertNotEqual(self.run_shell(self.entry,LABEL,'exec').returncode,0)
        self.assertEqual((self.stage/'selector-duplicate').read_text(),LABEL+'\n')
        self.assertNotEqual(self.quarantine().returncode,0)
