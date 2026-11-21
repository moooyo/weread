import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

import test_daily_render_device as daily
from qualified_submit import qualify
from run_daily_render import now
from test_daily_render_device import HERE, LABEL, MAIN, NONCE
from test_run_daily_render import START


class LibraryCollectorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        storage = tempfile.TemporaryDirectory()
        cls.addClassCleanup(storage.cleanup)
        cls.observer = Path(storage.name) / 'render-observer'
        subprocess.run(['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', HERE / 'render-observer.c',
                        '-o', cls.observer], check=True)

    def setUp(self):
        self.device = daily.DeviceTests(methodName='runTest')
        self.device.setUp()
        self.addCleanup(self.device.doCleanups)
        self.addCleanup(self.device.assert_protected)
        self.worker = dict(label=LABEL, measurement_build='temporary_measurement_build', operations=[])
        self.operation('admit', 'admitted')
        self.operation('stage', 'staged')
        self.operation('install', 'installed', payload=daily.archive(self.device.entries()))
        self.operation('prepare', 'prepared')
        self.device.invoke('arm', 'armed')
        self.root = self.device.root / 'diagnostic'
        self.root.mkdir(mode=0o700)
        self.request = self.root / 'request'
        self.deadline = time.clock_gettime_ns(time.CLOCK_MONOTONIC) + 50_000_000_000
        self.request.write_text(f'WEREAD_LIBRARY_REQUEST {LABEL} {self.deadline}\n')
        self.request.chmod(0o600)
        self.shell = ['busybox', 'sh'] if os.environ.get('WEREAD_TEST_BUSYBOX') else ['sh']

    def record(self, mode, **values):
        self.worker['operations'].append(dict(mode=mode, status=0, reason='', **values))

    def operation(self, mode, state=None, **values):
        raw = self.device.invoke(mode, state, **values)
        self.record(mode, receipt=json.loads(raw))
        return json.loads(raw)

    def clock_helper(self):
        stage = self.device.stage
        (stage / 'render-observer').rename(stage / 'fixture-scene')
        shutil.copyfile(self.observer, stage / 'clock-helper')
        (stage / 'clock-helper').chmod(0o700)
        (stage / 'render-observer').write_text('#!/bin/sh\ncase "$1" in --monotonic-ns) exec "${0%/*}/clock-helper" "$@";; '
                                               '*) exec "${0%/*}/fixture-scene" "$@";; esac\n')
        (stage / 'render-observer').chmod(0o700)

    def start(self):
        self.clock_helper()
        library = (HERE / 'library-submit.sh').read_text().replace('/var/tmp/weread-library-frame', str(self.root))
        device = self.device.program.read_text().split(MAIN)[0]
        driver = (device + library + '''
system_shell && shell_trusted=1
exec 6<"$3" 7<"$3/request"
library_submit_arm "$4" || exit 1
IFS= read -r proceed && [ "$proceed" = collect ] && library_submit_collect
''').replace('[ "$#" = 2 ]', '[ "$#" = 4 ]')
        child = subprocess.Popen([*self.shell, '-c', driver, 'witness', 'collect', LABEL, self.root, str(self.deadline)],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        def finish():
            if child.poll() is None:
                child.kill()
            child.communicate()
        self.addCleanup(finish)
        return child

    def test_clock_uses_the_same_linux_monotonic_domain(self):
        before = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
        sampled = int(subprocess.check_output([self.observer, '--monotonic-ns']))
        self.assertLessEqual(before, sampled)
        self.assertLessEqual(sampled, time.clock_gettime_ns(time.CLOCK_MONOTONIC))
        for deadline in ('0', '01', 'private', str(before), str(before + 100_000_000_000), str(2**63)):
            rejected = subprocess.run([self.observer, '--monotonic-ns', deadline], capture_output=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse(rejected.stdout)

    def test_foreign_or_insecure_request_is_not_armed(self):
        self.request.chmod(0o640)
        child = self.start()
        self.assertFalse(child.communicate(timeout=4)[0])
        self.assertNotEqual(child.returncode, 0)
        self.assertIn(LABEL, self.request.read_text())

    def launch(self):
        child = self.start()
        armed = json.loads(child.stdout.readline())
        self.record('arm', receipt=armed)
        observer = self.device.observe()
        self.record('observe_armed')
        down = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
        time.sleep(0.055)
        release = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
        tile = dict(START, down_before_ns=down, down_after_ns=down+1, up_before_ns=release-1, up_after_ns=release)
        tile = {key:tile[key] for key in 'ok mode clock hold_requested_ms down_confirmed up_sent buttons_clear_after target down_before_ns down_after_ns up_before_ns up_after_ns'.split()}
        (self.device.root / 'receipt').write_text(json.dumps(tile, separators=(',', ':')) + '\n')
        self.assertEqual(self.operation('tap'), tile)
        self.device.chain()
        self.operation('claim', 'claimed')
        observer.communicate(timeout=7)
        self.assertEqual(observer.returncode, 0)
        self.record('observe')
        self.operation('witness', 'witnessed')
        with self.request.open('r+b') as writer:
            fcntl.flock(writer, fcntl.LOCK_EX | fcntl.LOCK_NB)
            writer.truncate(0)
            submitted = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
            os.pwrite(writer.fileno(), f'WEREAD_LIBRARY_SUBMITTED {LABEL} {submitted} entries\n'.encode(), 0)
        return child, tile, submitted

    def test_ordinary_stage_release_and_original_request_reach_the_qualifier(self):
        child, tile, submitted = self.launch()
        output, errors = child.communicate(b'collect\n', timeout=8)
        self.assertEqual((child.returncode, errors), (0, b''))
        self.record('library_submit', receipt=json.loads(output))
        for mode, state in [('stop', 'stopped'), ('restore', 'restored'), ('close', 'closed'), ('absent', 'absent')]:
            self.operation(mode, state)
        self.worker.update(released=True, recovery_required=False, owner_and_token_checked=True, released_at_utc=now(),
                           synthetic_release_ns=[tile['up_before_ns'], tile['up_after_ns']])
        self.assertEqual(qualify(self.worker)['elapsed_interval_ns'], [submitted-tile['up_after_ns'], submitted-tile['up_before_ns']])

    def test_new_request_with_identical_marker_is_not_the_pinned_file(self):
        child, _, _ = self.launch()
        self.request.rename(self.root / 'original')
        shutil.copy(self.root / 'original', self.request)
        self.assertFalse(child.communicate(b'collect\n', timeout=6)[0])
        self.assertNotEqual(child.returncode, 0)

    def test_new_generation_cannot_bind(self):
        child, _, _ = self.launch()
        (self.device.lock / 'generation').write_text('2:' + NONCE + '\n')
        self.assertFalse(child.communicate(b'collect\n', timeout=6)[0])
        self.assertNotEqual(child.returncode, 0)

    def test_partial_or_multiline_marker_never_escapes(self):
        child, _, _ = self.launch()
        with self.request.open('ab') as writer:
            writer.write(b'private extra line\n')
        output, errors = child.communicate(b'collect\n', timeout=6)
        self.assertEqual((output, errors), (b'', b''))
        self.assertNotEqual(child.returncode, 0)
