from copy import deepcopy
import fcntl
import json
import os
import select
import subprocess
import time
import unittest

import test_daily_render_device as daily
from qualified_submit import ISOLATED_SEQUENCE, Refused, qualify
from test_qualified_submit import receipt


class StockRequestTests(unittest.TestCase):
    def setUp(self):
        self.device = daily.DeviceTests(methodName='runTest')
        self.device.setUp()
        self.addCleanup(self.device.doCleanups)
        self.device.setup_stage()
        self.root = self.device.root / 'var/tmp'
        self.root.mkdir(mode=0o700)
        self.root.chmod(0o2700)
        self.fixed = self.root / 'weread-library-frame'
        self.request = self.fixed / 'request'
        self.draft = self.root / ('.weread-library-frame-' + daily.LABEL)
        original = (daily.HERE / 'render-observer.c').read_text()
        compiled = self.device.root / 'observer.c'
        compiled.write_text(original.replace('/var/tmp/', str(self.root) + '/'))
        self.observer = self.device.stage / 'render-observer'
        native = self.device.root / 'render-observer'
        subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', '-I', daily.HERE,
                        compiled, '-o', native], check=True)
        native.replace(self.observer)
        device = self.device.program.read_text().split(daily.MAIN)[0]
        includes = '\n'.join((daily.HERE / leaf).read_text() for leaf in ('library-submit.sh', 'stock-library-request.sh'))
        self.driver = device + includes.replace('/var/tmp/', str(self.root) + '/') + '''
system_shell && shell_trusted=1
witness() { current=$(gui) && matches "$stage/session" "$current"; }
case "$mode" in
    request) stage_lock && stock_request_serve;;
    tap) stage_lock && stock_request_active && tile tap;;
    cleanup) stage_lock && stock_request_cleanup;;
    absent) stock_request_absent;;
    *) exit 64;;
esac
'''
        self.shell = ['busybox', 'sh'] if os.getenv('WEREAD_TEST_BUSYBOX') else ['sh']

    def call(self, mode):
        return subprocess.run([*self.shell, '-c', self.driver, 'stock', mode, daily.LABEL], capture_output=True, timeout=5)

    def start(self, armed=True):
        child = subprocess.Popen([*self.shell, '-c', self.driver, 'stock', 'request', daily.LABEL],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        def finish():
            if child.poll() is None:
                child.kill()
            child.communicate()
        self.addCleanup(finish)
        if not armed:
            return child, None
        self.assertTrue(select.select([child.stdout], [], [], 5)[0])
        line = child.stdout.readline()
        self.assertTrue(line, child.communicate() if not line else None)
        return child, json.loads(line)

    def test_pinned_original_stage_click_and_app_marker_qualify(self):
        child, arm = self.start()
        self.assertEqual(self.fixed.stat().st_mode & 0o7777, 0o700)
        self.assertEqual(self.request.stat().st_mode & 0o7777, 0o600)
        self.assertNotEqual(self.call('cleanup').returncode, 0)
        down = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
        time.sleep(0.052)
        up = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
        tap = dict(json.loads(daily.RELEASE), down_before_ns=down, down_after_ns=down+1, up_before_ns=up-1, up_after_ns=up)
        (self.device.root / 'receipt').write_text(json.dumps(tap, separators=(',', ':')) + '\n')
        outcome = self.call('tap')
        self.assertEqual(outcome.returncode, 0, outcome.stderr)
        self.device.chain()
        self.device.invoke('claim', 'claimed')
        with self.request.open('r+b') as writer:
            fcntl.flock(writer, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(writer.read(), f'WEREAD_LIBRARY_REQUEST {daily.LABEL} {arm["request_deadline_ns"]}\n'.encode())
            writer.truncate(0)
            sent = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
            os.pwrite(writer.fileno(), f'WEREAD_LIBRARY_SUBMITTED {daily.LABEL} {sent} entries\n'.encode(), 0)
        output, errors = child.communicate(b'collect\n', timeout=5)
        self.assertEqual((child.returncode, errors), (0, b''))
        self.assertEqual(self.call('cleanup').returncode, 0)
        self.assertEqual(self.call('absent').returncode, 0)
        self.device.cleanup_stage()
        report = receipt()
        rows = {mode:{'mode':mode, 'status':0, 'reason':''} for mode in ISOLATED_SEQUENCE}
        rows['arm']['receipt'] = {'state':'armed'}
        rows['request_armed']['receipt'], rows['tap']['receipt'], rows['library_submit']['receipt'] = arm, json.loads(outcome.stdout), json.loads(output)
        report.update(protocol='isolated_stock_launch', measurement_build='temporary_measurement_build', fixture='disposable_public',
                      operations=list(rows.values()), synthetic_release_ns=[up-1, up])
        self.assertEqual(qualify(report)['elapsed_interval_ns'], [sent-up, sent-up+1])
        for change in ({'fixture':'private'}, {'protocol':'other'}, {'measurement_build':'ordinary_installation'}, {'observation':{}}):
            with self.subTest(change=change), self.assertRaises(Refused):
                qualify(report | change)
        for mode in ('activate', 'request_armed', 'witness'):
            changed = deepcopy(report)
            changed['operations'] = [row for row in changed['operations'] if row['mode'] != mode]
            with self.assertRaises(Refused):
                qualify(changed)

    def test_cancel_or_eof_cleans_without_input(self):
        child, _ = self.start()
        output, errors = child.communicate(timeout=5)
        self.assertEqual((child.returncode, output, errors), (0, b'', b''))
        self.assertEqual(self.call('absent').returncode, 0)
        self.assertFalse((self.device.stage / 'tile-dispatch-intent').exists())
        self.assertNotEqual(self.call('tap').returncode, 0)

    def test_existing_fixed_directory_is_never_reused(self):
        self.fixed.mkdir()
        before = self.fixed.stat()
        child, _ = self.start(armed=False)
        self.assertEqual(child.communicate(timeout=4)[0], b'')
        self.assertNotEqual(child.returncode, 0)
        self.assertFalse(self.draft.exists())
        self.assertEqual(before.st_ino, self.fixed.stat().st_ino)

    def test_replaced_inode_refuses_collection_and_does_not_remove_new_file(self):
        child, _ = self.start()
        self.request.rename(self.root / 'pinned-old')
        self.request.write_text('third-party file\n')
        _, errors = child.communicate(b'cancel\n', timeout=4)
        self.assertFalse(errors)
        self.assertNotEqual(child.returncode, 0)
        self.assertNotEqual(self.call('cleanup').returncode, 0)
        self.assertEqual(self.request.read_text(), 'third-party file\n')

    def test_unrecognized_in_place_data_is_not_unlinked(self):
        child, _ = self.start()
        self.request.write_text('unrelated content\n')
        self.assertFalse(child.communicate(b'cancel\n', timeout=4)[0])
        self.assertNotEqual(child.returncode, 0)
        self.assertNotEqual(self.call('cleanup').returncode, 0)
        self.assertEqual(self.request.read_text(), 'unrelated content\n')

    def test_sigkill_after_arming_can_be_cleaned_without_another_tap(self):
        child, _ = self.start()
        child.kill()
        child.communicate(timeout=5)
        self.assertEqual(self.call('cleanup').returncode, 0)
        self.assertEqual(self.call('absent').returncode, 0)
        self.assertFalse((self.device.stage / 'tile-dispatch-intent').exists())

    def test_malformed_long_input_and_expired_command_refuse(self):
        child, _ = self.start()
        output, _ = child.communicate(b'collect\nprivate\n', timeout=5)
        self.assertFalse(output)
        self.assertNotEqual(child.returncode, 0)
        self.assertEqual(self.call('absent').returncode, 0)
        for raw, offset in [(b'collect\n', -1), (b'collect', 1_000_000_000), (b'cancel\nextra\n', 1_000_000_000)]:
            deadline = time.clock_gettime_ns(time.CLOCK_MONOTONIC) + offset
            result = subprocess.run([self.observer, '--request-stdin', str(deadline)], input=raw, capture_output=True, timeout=2)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(result.stdout)
