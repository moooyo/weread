from pathlib import Path
import os
import subprocess
import tempfile
import unittest


HERE = Path(__file__).resolve().parent
IDENTITY = '63:87'
GENERATION = '1:fixture_generation'


class FixedForegroundTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.addCleanup(self.check_leaks)
        self.proc = self.root / 'proc'
        self.tmp = self.root / 'tmp'
        self.tmp.mkdir()
        self.stage = self.root / 'candidate'
        self.out = self.stage / 'out'
        self.out.mkdir(parents=True)
        self.lock = self.tmp / 'weread-launcher.lock'
        self.lock.mkdir()
        self.home = self.stage / 'B'
        (self.home / 'app').mkdir(parents=True)
        self.app = self.home / 'app/weread'
        self.app.touch()
        self.gui = self.proc / '63'
        self.fds = self.gui / 'fd'
        self.fds.mkdir(parents=True)
        (self.gui / 'exe').symlink_to(self.app)
        self.stat(self.gui, 'S', 87)
        self.manager = self.proc / '97'
        self.manager.mkdir()
        self.stat(self.manager, 'T', 90)
        self.put('power', 'active')
        self.put('wm', '97')
        self.put('proc/uptime', '1200.00 600.00')
        self.put('tmp/weread-launcher.lock/generation', GENERATION)
        self.put('tmp/weread-launcher.lock/application-owned', GENERATION + ':' + IDENTITY)
        self.put('tmp/weread-launcher.lock/application-pid', '63')
        self.put('tmp/weread-launcher.lock/application-birth', '87')
        self.arguments = [str(self.app), '--kindle', '--device', 'scribe', '--data-dir', str(self.home / 'data'),
                          '--demo', '--offline', '--screen', 'library', '--exit-after-ms', '35000',
                          '--capture', str(self.out / 'final.png')]
        (self.gui / 'cmdline').write_bytes(b'\x00'.join(arg.encode() for arg in self.arguments) + b'\x00')
        (self.fds / '3').symlink_to('/dev/fb0')
        (self.fds / '4').symlink_to('/dev/input/event2')
        self.output = b'SHELF 123000000000 123000000014 1 grid\n'
        self.helper = self.stage / 'shelf-scene'
        self.helper.write_text('#!/bin/sh\necho sample >>"$ROOT/reads"\n'
                               'if [ -f "$ROOT/at_capture" ]; then sh -e "$ROOT/at_capture"; fi\n'
                               'printf "SHELF 123000000000 123000000014 1 grid\\n"\n')
        self.helper.chmod(0o755)

    def put(self, name, value):
        (self.root / name).write_text(value + '\n')

    @staticmethod
    def stat(folder, state, birth):
        (folder / 'stat').write_text(' '.join([folder.name, '(fixture)', state, *['0'] * 18, str(birth)]) + '\n')

    def check_leaks(self):
        if hasattr(self, 'result'):
            self.assertNotIn(GENERATION.encode(), self.result.stderr)
            self.assertNotIn(str(self.root).encode(), self.result.stderr)

    def execute(self):
        common = (HERE / 'device.sh').read_text()
        device = (HERE / 'orientation-device.sh').read_text()
        source = common[common.index('fail() {'):common.index('owned() {')]
        source += common[common.index('gui_owned() {'):common.index('archive_public() {')]
        source += device[device.index('gui_args() {'):device.index('case_admission() {')].replace('dwell=12000', 'dwell=35000')
        shared = (HERE / 'pose_visit.sh').read_text()
        source += shared[shared.index('power_state() {'):shared.index('public_foreground() {')]
        source += (HERE / 'assisted-fixed-foreground.sh').read_text()
        source = source.replace('"/tmp/', '"' + str(self.tmp) + '/').replace('/proc/', str(self.proc) + '/')
        fixtures = '''
stage=$ROOT/candidate; out=$stage/out; home=$stage/B; lock=$ROOT/tmp/weread-launcher.lock; name=B-library
gui=63; birth=87; identity=63:87; generation=1:fixture_generation
power_state() { cat "$ROOT/power"; }
pidof() {
    if [ -f "$ROOT/pidof_$1" ]; then printf 'injected pidof error\n' >&2; return "$(cat "$ROOT/pidof_$1")"; fi
    case "$1" in awesome) cat "$ROOT/wm";; aidoku|bilicomics) [ -f "$ROOT/$1" ];; *) return 2;; esac
}
usleep() {
    [ "$1" = 100000 ] || return 1
    echo wait >>"$ROOT/waits"
    if [ -f "$ROOT/at_wait" ]; then sh -e "$ROOT/at_wait"; fi
}
mv() { [ ! -e "$ROOT/reject_move" ] && command mv "$@"; }
fixed_capture_shelf
'''
        self.result = subprocess.run(['sh', '-ec', source + fixtures], env=dict(os.environ, ROOT=str(self.root)),
                                     capture_output=True, timeout=5)
        return self.result

    def refused(self, reason, wanted_reads=0, wanted_waits=0):
        result = self.execute()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((result.stdout, result.stderr), (b'', ('refused ' + reason + '\n').encode()))
        self.assertEqual(self.count('reads'), wanted_reads)
        self.assertEqual(self.count('waits'), wanted_waits)
        self.assertFalse((self.out / 'shelf').exists())
        self.assertFalse((self.out / 'shelf.next').exists())

    def count(self, name):
        path = self.root / name
        return len(path.read_text().splitlines()) if path.exists() else 0

    def test_owned_scene_commits_once_without_wait(self):
        result = self.execute()
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b'', b''))
        self.assertEqual((self.out / 'shelf').read_bytes(), self.output)
        self.assertEqual((self.count('reads'), self.count('waits')), (1, 0))

    def test_short_busy_start_rechecks_the_complete_foreground_before_read(self):
        self.stat(self.gui, 'D', 87)
        self.put('at_wait', 'sed -i "s/(fixture) D /(fixture) S /" "$ROOT/proc/63/stat"')
        result = self.execute()
        self.assertEqual((result.returncode, result.stderr), (0, b''))
        self.assertEqual((self.out / 'shelf').read_bytes(), self.output)
        self.assertEqual((self.count('reads'), self.count('waits')), (1, 1))

    def test_busy_is_never_an_admissible_final_state(self):
        self.stat(self.gui, 'D', 87)
        self.refused('pre_task_busy', wanted_waits=10)

    def test_a_late_ready_probe_does_not_pass_the_retry_deadline(self):
        self.stat(self.gui, 'D', 87)
        self.put('at_wait', 'echo "1201.21 600.00" >"$ROOT/proc/uptime"\n'
                            'sed -i "s/(fixture) D /(fixture) R /" "$ROOT/proc/63/stat"')
        self.refused('pre_task_busy', wanted_waits=1)

    def test_no_clock_does_not_start_a_wait(self):
        self.stat(self.gui, 'D', 87)
        self.put('proc/uptime', 'unknown')
        self.refused('pre_clock')

    def test_changed_argv_while_waiting_is_a_hard_refusal(self):
        self.stat(self.gui, 'D', 87)
        self.put('at_wait', 'echo other-application >"$ROOT/proc/63/cmdline"')
        self.refused('pre_gui', wanted_waits=1)

    def test_peer_and_generation_and_owner_are_rechecked_during_wait(self):
        self.stat(self.gui, 'D', 87)
        for path, reason in [('aidoku', 'pre_peer_process'), ('tmp/weread-launcher.lock/generation', 'pre_generation'),
                             ('tmp/weread-launcher.lock/application-owned', 'pre_owner')]:
            with self.subTest(reason=reason):
                target = self.root / path
                previous = target.read_bytes() if target.exists() else None
                self.put('at_wait', 'echo changed >"$ROOT/' + path + '"')
                self.refused(reason, wanted_waits=1)
                (self.root / 'waits').unlink()
                if previous is None:
                    target.unlink()
                else:
                    target.write_bytes(previous)

    def test_initial_guards_distinguish_each_failed_resource(self):
        self.stat(self.gui, 'D', 87)
        for path, content, reason in [('power', 'screenSaver', 'pre_power'), ('wm', '', 'pre_manager'),
                                      ('proc/97/stat', '97 (awesome) R', 'pre_manager_state'),
                                      ('tmp/bilicomics-launcher.lock', '', 'pre_peer_lock')]:
            with self.subTest(reason=reason):
                target = self.root / path
                previous = target.read_bytes() if target.exists() else None
                self.put(path, content)
                self.refused(reason)
                if previous is None:
                    target.unlink()
                else:
                    target.write_bytes(previous)
        (self.fds / '3').unlink()
        self.refused('pre_display')
        (self.fds / '3').symlink_to('/dev/fb0')
        (self.fds / '4').unlink()
        self.refused('pre_touch')

    def test_changed_pid_birth_with_matching_new_argv_is_not_accepted(self):
        self.put('tmp/weread-launcher.lock/application-birth', '88')
        self.stat(self.gui, 'S', 88)
        self.refused('pre_identity')

    def test_peer_lookup_errors_never_mean_no_peer(self):
        for peer in ('aidoku', 'bilicomics'):
            for status in (2, 127):
                with self.subTest(peer=peer, status=status):
                    self.put('pidof_' + peer, str(status))
                    self.refused('pre_peer_process')
                    (self.root / ('pidof_' + peer)).unlink()
        self.put('at_capture', 'echo 127 >"$ROOT/pidof_bilicomics"')
        self.refused('post_peer_process', wanted_reads=1)

    def test_stopped_gui_is_never_retried(self):
        self.stat(self.gui, 'T', 87)
        self.refused('pre_task')

    def test_loss_during_read_discards_output_and_never_retries(self):
        for alteration, reason in [('rm "$ROOT/proc/63/fd/4"', 'post_touch'),
                                   ('sed -i "s/(fixture) S /(fixture) D /" "$ROOT/proc/63/stat"', 'post_task_busy'),
                                   ('touch "$ROOT/bilicomics"', 'post_peer_process')]:
            with self.subTest(reason=reason):
                self.put('at_capture', alteration)
                self.refused(reason, wanted_reads=1)
                (self.root / 'reads').unlink()
                self.stat(self.gui, 'S', 87)
                if not (self.fds / '4').is_symlink():
                    (self.fds / '4').symlink_to('/dev/input/event2')

    def test_publication_failure_is_not_reported_as_ownership_loss(self):
        self.put('reject_move', '')
        self.refused('scene_publish', wanted_reads=1)

    def test_failed_observer_results_in_no_partial_sample(self):
        self.helper.write_text('#!/bin/sh\necho partial-output; exit 1\n')
        result = self.execute()
        self.assertEqual((result.returncode, result.stderr), (0, b''))
        self.assertEqual((self.out / 'shelf').read_bytes(), b'not_observed\n')


if __name__ == '__main__':
    unittest.main()
