import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

from assisted_wake_samples import render_observer


HERE = Path(__file__).resolve().parent
LABEL = '019d6f1a12b34cde'
GENERATION = '1:' + 'a' * 32
CYCLE = GENERATION + ':1'


class AssistedWakeObserverTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.stage = self.root / ('mnt/us/.weread-orientation-' + LABEL)
        self.out = self.stage / 'out/B-reader'
        self.home = self.stage / 'B'
        self.app = self.home / 'app/weread'
        self.lock = self.root / 'tmp/weread-launcher.lock'
        self.bin = self.root / 'bin'
        self.proc = self.root / 'proc'
        self.manager = self.proc / '97'
        self.gui = self.proc / '63'
        for directory in (self.out / 'entered', self.app.parent, self.lock, self.manager, self.gui / 'fd', self.bin):
            directory.mkdir(parents=True)
        self.source = (HERE / 'assisted-wake-observe.sh').read_text(encoding='ascii')
        self.helper(self.bin / 'lipc-get-prop', '[ "$*" = "-s com.lab126.powerd state" ]; cat "$ROOT/power"')
        self.helper(self.bin / 'pidof', '''case "$1" in
            awesome) cat "$ROOT/manager";; aidoku|bilicomics) if [ -f "$ROOT/peer-$1" ]; then exit "$(cat "$ROOT/peer-$1")"; else exit 1; fi;;
            *) exit 2;; esac''')
        self.helper(self.stage / 'pose-snapshot', '''[ "$*" = --display-sample ]; echo read >>"$ROOT/reads"
            if [ -f "$ROOT/mutate" ]; then sh -e "$ROOT/mutate"; fi
            cat "$ROOT/sensor"; [ ! -e "$ROOT/helper-failed" ]''')
        self.app.touch()
        self.put(self.stage / 'owner', LABEL)
        self.put(self.out / 'entered/arm', 'B-reader')
        self.put(self.lock / 'nonce', GENERATION[2:])
        self.pose('D:1')
        for marker in ('generation', 'managed-sleep-available', 'state-open'):
            self.put(self.lock / marker, GENERATION)
        self.bind('63', '73')
        (self.gui / 'exe').symlink_to(self.app)
        (self.gui / 'cmdline').write_bytes(b'\0'.join(arg.encode() for arg in self.arguments()) + b'\0')
        self.task(self.gui, 'S', '73')
        self.task(self.manager, 'T', '54')
        (self.gui / 'fd/4').symlink_to('/dev/fb0')
        (self.gui / 'fd/5').symlink_to('/dev/input/event2')
        self.put(self.root / 'manager', '97')
        self.put(self.root / 'power', 'active')
        self.sample(16, 1)

    @staticmethod
    def put(path, text):
        path.write_text(text + '\n', encoding='ascii')

    @staticmethod
    def helper(path, body):
        path.write_text('#!/bin/sh\nset -eu\n' + body + '\n', encoding='ascii')
        path.chmod(0o755)

    @staticmethod
    def task(directory, state, birth):
        (directory / 'stat').write_text(' '.join([directory.name, '(safe fixture)', state, *['0'] * 18, birth]) + '\n')

    def arguments(self):
        return [str(self.app), '--kindle', '--device', 'scribe', '--data-dir', str(self.home / 'data'), '--demo',
                '--offline', '--screen', 'reader', '--exit-after-ms', '105000', '--capture', str(self.out / 'final.png')]

    def bind(self, pid, birth, *, pin=True):
        self.put(self.lock / 'application-pid', pid)
        self.put(self.lock / 'application-birth', birth)
        self.put(self.lock / 'application-owned', f'{GENERATION}:{pid}:{birth}')
        if pin:
            self.put(self.out / 'wake-owner', f'{GENERATION}:{pid}:{birth}')

    def pose(self, value):
        self.put(self.out / 'portrait', value)
        self.put(self.out / 'portrait-launch', value)
        self.put(self.lock / 'native-portrait-calibration', GENERATION + ':' + value)

    def sample(self, raw, rotation):
        self.put(self.root / 'sensor', f'sample 1233456789988777 1233456789988779 {raw} {rotation}')

    def asleep(self):
        for marker in ('managed-sleep-request', 'managed-sleep-ready'):
            self.put(self.lock / marker, CYCLE)
        for marker in ('sleep-request', 'resume-ready'):
            self.put(self.lock / marker, GENERATION)
        self.task(self.gui, 'T', '73')
        self.put(self.root / 'power', 'screenSaver')

    def awake(self):
        self.asleep()
        for marker in ('managed-wake-request', 'managed-wake-ready'):
            self.put(self.lock / marker, CYCLE)
        self.task(self.gui, 'S', '73')
        self.put(self.root / 'power', 'active')

    def execute(self, phase, *, command='wake_observe', real_proc=False, rendered=False):
        observer = render_observer().decode('ascii') if rendered or command == 'wake_observe_any' else self.source
        source = observer.replace('/mnt/us/.weread-orientation-', str(self.root) + '/mnt/us/.weread-orientation-')
        source = source.replace('"/tmp/$wake_peer', '"' + str(self.root) + '/tmp/$wake_peer')
        if not real_proc:
            source = source.replace('/proc/', str(self.proc) + '/')
        else:
            source = source.replace(' /dev/fb0)', ' "$ROOT/dev/fb0")').replace(' /dev/input/event', ' "$ROOT"/dev/input/event')
        harness = '''
plain() { [ -f "$1" ] && [ ! -L "$1" ] && [ "$(readlink -f "$1")" = "$1" ]; }
absent() { [ ! -e "$1" ] && [ ! -L "$1" ]; }
pidof() { "$ROOT/bin/pidof" "$@"; }
label=$1; stage=$ROOT/mnt/us/.weread-orientation-$label; home=$stage/B; out=$stage/out/B-reader; name=B-reader
lock=$ROOT/tmp/weread-launcher.lock
"$2" "$3"
'''
        result = subprocess.run(['sh', '-ec', source + harness, 'fake-reader', LABEL, command, phase],
                                env=dict(os.environ, PATH=str(self.bin) + ':' + os.environ['PATH'], ROOT=str(self.root)),
                                capture_output=True, timeout=5)
        self.assertNotIn(GENERATION.encode(), result.stdout + result.stderr)
        return result

    def accepted(self, phase, raw, rotation, cal, **options):
        result = self.execute(phase, **options)
        self.assertEqual((result.returncode, result.stderr, result.stdout), (0, b'',
                         f'WAKE_OBS {phase} 1233456789988777 1233456789988779 {raw} {rotation} {cal}\n'.encode()))

    def refused(self, phase, code=None, **options):
        result = self.execute(phase, **options)
        self.assertNotEqual(result.returncode, 0)
        if code is not None:
            self.assertEqual(result.returncode, code)
        self.assertEqual(result.stdout, b'')

    def test_full_turn_uses_four_bracketed_reads_and_never_requires_a_rotated_sleep_scanout(self):
        self.accepted('pre', 16, 1, 'D:1')
        self.asleep()
        self.accepted('sleep-pre', 16, 1, 'none')
        self.sample(17, 1)
        self.accepted('sleep-post', 17, 1, 'none')
        self.awake()
        self.sample(15, 3)
        self.put(self.lock / 'managed-native-portrait-calibration', CYCLE + ':U:3')
        self.accepted('post', 15, 3, 'U:3')
        self.assertEqual((self.root / 'reads').read_text().splitlines(), ['read'] * 4)

    def test_reverse_turn_and_absent_native_wake_calibration_are_reported_separately(self):
        self.pose('U:3')
        self.sample(15, 3)
        self.accepted('pre', 15, 3, 'U:3')
        self.asleep()
        self.accepted('sleep-pre', 15, 3, 'none')
        self.sample(18, 3)
        self.accepted('sleep-post', 18, 3, 'none')
        self.awake()
        self.sample(16, 1)
        self.accepted('post', 16, 1, 'none')

    def test_active_opposite_sample_cannot_substitute_for_in_sleep_change(self):
        self.asleep()
        self.refused('sleep-post', 47)
        self.refused('sleep-post', 47, rendered=True)
        self.accepted('sleep-post', 16, 1, 'none', command='wake_observe_any')
        self.awake()
        self.sample(15, 3)
        self.refused('sleep-post')
        self.refused('sleep-post', command='wake_observe_any')
        self.accepted('post', 15, 3, 'none', command='wake_observe_any')

    def test_screensaver_samples_never_preselect_pressures_at_either_boundary(self):
        self.asleep()
        for phase in ('sleep-pre', 'sleep-post'):
            for raw in (15, 16, 17, 18):
                with self.subTest(phase=phase, raw=raw):
                    self.sample(raw, 1)
                    self.accepted(phase, raw, 1, 'none', command='wake_observe_any')

    def test_unchanged_resumed_pressure_is_still_a_valid_resident_observation(self):
        self.pose('U:3')
        self.awake()
        self.put(self.lock / 'managed-native-portrait-calibration', CYCLE + ':U:3')
        for raw, scanout in ((15, 3), (17, 1), (16, 3), (18, 1)):
            with self.subTest(raw=raw, scanout=scanout):
                self.sample(raw, scanout)
                self.accepted('post', raw, scanout, 'U:3', command='wake_observe_any')

    def test_sleep_authority_needs_identity_and_state_not_a_readable_or_opposite_sensor(self):
        self.asleep()
        self.helper(self.stage / 'pose-snapshot', 'exit 2')
        result = self.execute('sleep-post', command='wake_guard', rendered=True)
        self.assertEqual((result.returncode, result.stdout, result.stderr), (0, b'', b''))
        self.refused('sleep-post', 45, command='wake_observe_any')
        self.put(self.root / 'power', 'active')
        self.refused('sleep-post', 42, command='wake_guard')
        self.put(self.root / 'power', 'screenSaver')
        self.put(self.lock / 'application-owned', 'different')
        self.refused('sleep-post', 24, command='wake_guard')

    def test_actual_sampling_still_refuses_unanchored_initial_reader_and_stale_post_identity(self):
        self.sample(17, 3)
        self.refused('pre', 47, command='wake_observe_any')
        self.awake()
        self.bind('63', '88', pin=False)
        self.refused('post', 24, command='wake_observe_any')

    def test_no_sleep_or_turn_cue_for_first_screen_or_marker_alone(self):
        self.refused('sleep-pre')
        self.asleep()
        for path, invalid in [(self.root / 'power', 'active'), (self.gui / 'stat', '63 (weread) S 2'),
                              (self.lock / 'managed-sleep-ready', GENERATION + ':2'),
                              (self.lock / 'resume-ready', 'other'), (self.lock / 'sleep-request', 'other')]:
            with self.subTest(path=path.name):
                saved = path.read_bytes()
                self.put(path, invalid)
                self.refused('sleep-pre')
                path.write_bytes(saved)
        for marker in ('managed-wake-request', 'managed-wake-request.pending', 'managed-wake-ready', 'ordinary-exit'):
            with self.subTest(marker=marker):
                path = self.lock / marker
                self.put(path, CYCLE)
                self.refused('sleep-pre')
                path.unlink()
        self.assertFalse((self.root / 'reads').exists())

    def test_transition_during_suspended_read_discards_rather_than_publishes(self):
        self.asleep()
        self.sample(17, 1)
        self.put(self.root / 'mutate', 'echo active >"$ROOT/power"')
        self.refused('sleep-post', 42)
        self.assertEqual((self.root / 'reads').read_text(), 'read\n')
        self.put(self.root / 'power', 'screenSaver')
        self.put(self.root / 'mutate', 'touch "$ROOT/tmp/weread-launcher.lock/managed-wake-request.pending"')
        self.refused('sleep-post', 39)

    def test_newly_published_advertisement_is_mandatory_and_not_a_checkpoint_or_half_write(self):
        available = self.lock / 'managed-sleep-available'
        available.unlink()
        self.put(self.lock / 'managed-sleep-available.pending', GENERATION)
        self.refused('pre', 26)
        self.put(available, GENERATION + ':1')
        self.refused('pre', 26)
        self.assertFalse((self.root / 'reads').exists())
        (self.lock / 'managed-sleep-available.pending').replace(available)
        self.accepted('pre', 16, 1, 'D:1')
        state = self.lock / 'state-open'
        state.write_bytes(GENERATION.encode())
        self.refused('pre', 25)

    def test_errors_retain_their_failure_site_without_exposing_record_values(self):
        for original, name, value, code in [('lock', 'application-owned', 'private_fixture_value', 24),
                                           ('lock', 'generation', '2:' + 'a' * 32, 23),
                                           ('lock', 'sleep-request.pending', '', 37),
                                           ('out', 'wake-owner', 'malformed', 22), ('out', 'guard', '', 21),
                                           ('out', 'portrait-launch', 'U:1', 31), ('stage', 'owner', 'wrong', 20),
                                           ('gui', 'stat', '63 (fake) T', 28), ('manager', 'stat', '97 (wm) R', 34)]:
            with self.subTest(name=name):
                path = getattr(self, original) / name
                saved = path.read_bytes() if path.is_file() else None
                self.put(path, value)
                self.refused('pre', code)
                if saved is None:
                    path.unlink()
                else:
                    path.write_bytes(saved)
        self.task(self.gui, 'D', '73')
        self.refused('pre', 32)
        self.task(self.gui, 'T', '73')
        self.refused('pre', 33)
        self.assertFalse((self.root / 'reads').exists())

    def test_actual_helper_failure_malformed_sample_and_deadline_are_not_mislabeled_as_a_busy_sensor(self):
        self.helper(self.stage / 'pose-snapshot', 'printf "private_partial_output\\n"; exit 2')
        self.refused('pre', 45)
        self.helper(self.stage / 'pose-snapshot', 'printf "private_partial_output\\n"')
        self.refused('pre', 46)
        self.helper(self.stage / 'pose-snapshot', 'exec sleep 5')
        self.refused('pre', 49)
        (self.stage / 'pose-snapshot').chmod(0o600)
        self.refused('pre', 43)
        self.helper(self.bin / 'lipc-get-prop', 'exit 1')
        self.refused('pre', 41)

    def test_closed_reason_dictionary_cannot_echo_non_code_values(self):
        reasons = ('stage_scope entry_state owner_record generation application_identity state_open managed_available '
                   'executable process_birth arguments peer launch_calibration task_busy task_state manager display input '
                   'phase_markers sleep_cycle wake_cycle wake_calibration power_query power_state sensor_helper sensor_busy '
                   'sensor_query sensor_format pose_disagrees unstable_scope sensor_timeout').split()
        for code, expected in [*enumerate(reasons, 20), (0, 'observer_unknown'), (1, 'observer_unknown'),
                               ('private_fixture_value', 'observer_unknown')]:
            with self.subTest(code=code):
                result = self.execute(str(code), command='wake_reason')
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, (expected + '\n').encode(), b''))

    def test_cold_replacement_wrong_generation_and_changed_public_route_fail_closed(self):
        for path, value in [(self.lock / 'generation', '2:' + 'a' * 32), (self.lock / 'application-pid', '64'),
                            (self.lock / 'application-owned', GENERATION + ':63:198'), (self.lock / 'application-birth', '74'),
                            (self.out / 'entered/arm', 'A-reader'), (self.stage / 'owner', 'different_tag'),
                            (self.out / 'wake-owner', GENERATION + ':63:73\nrepeated')]:
            with self.subTest(path=path.name):
                saved = path.read_bytes()
                self.put(path, value)
                self.refused('pre')
                path.write_bytes(saved)
        cmdline = self.gui / 'cmdline'
        cmdline.write_bytes(cmdline.read_bytes().replace(b'reader\0--exit-after', b'library\0--exit-after'))
        self.refused('pre')
        self.assertFalse((self.root / 'reads').exists())

    def test_closed_front_or_peer_lookup_error_never_authorizes_a_page_capture(self):
        for path in [self.gui / 'fd/4', self.gui / 'fd/5']:
            target = path.readlink()
            path.unlink()
            self.refused('pre', command='wake_guard')
            path.symlink_to(target)
        self.task(self.manager, 'R', '54')
        self.refused('pre', command='wake_guard')
        self.task(self.manager, 'T', '54')
        self.put(self.root / 'peer-bilicomics', '2')
        self.refused('pre', command='wake_guard')
        (self.root / 'peer-bilicomics').unlink()
        self.put(self.root / 'tmp/aidoku-launcher.lock', 'peer')
        self.refused('pre', command='wake_guard')
        self.assertFalse((self.root / 'reads').exists())

    def test_post_requires_wake_ready_of_first_cycle_and_a_live_public_view(self):
        self.awake()
        self.sample(17, 3)
        for marker, value in [('managed-sleep-request', GENERATION + ':2'), ('managed-wake-ready', 'stale'),
                              ('managed-wake-request', GENERATION + ':2'), ('managed-native-portrait-calibration', GENERATION + ':2:U:3')]:
            with self.subTest(marker=marker):
                path = self.lock / marker
                saved = path.read_bytes() if path.exists() else None
                self.put(path, value)
                self.refused('post')
                if saved is None:
                    path.unlink()
                else:
                    path.write_bytes(saved)
        self.task(self.gui, 'T', '73')
        self.refused('post')
        self.assertFalse((self.root / 'reads').exists())

    def test_unexpected_failures_and_malformed_sample_do_not_emit_a_partial_row(self):
        for value in ('sample 12345678912345679 12345678912345678 16 1', 'sample 0 1 16 1',
                      'sample 2 9223372036854775808 16 1', 'sample 2 3 0 1', 'sample 2 3 16 4',
                      'sample 2 3 16 1\nsample 4 5 16 1', 'sample 10 11 17 3'):
            with self.subTest(value=value):
                self.put(self.root / 'sensor', value)
                self.refused('pre')
        self.sample(16, 1)
        self.put(self.root / 'helper-failed', 'yes')
        self.refused('pre')
        self.refused('invalid')

    def test_real_linux_stop_continue_preserves_the_bound_process_but_not_a_cold_replacement(self):
        devices = self.root / 'dev'
        (devices / 'input').mkdir(parents=True)
        for path in ('fb0', 'input/event2'):
            (devices / path).touch()
        source = '#include <fcntl.h>\n#include <stdio.h>\n#include <unistd.h>\nint main(void) {'
        for path in ('fb0', 'input/event2'):
            source += f'if (open("{devices / path}", O_RDONLY) < 0) return 1;'
        source += 'puts("ready"); fflush(stdout); for (;;) pause();}'
        subprocess.run(['cc', '-x', 'c', '-', '-o', self.app], input=source.encode(), check=True, capture_output=True, timeout=10)

        def launch(arguments):
            child = subprocess.Popen(arguments, stdout=subprocess.PIPE)
            def close():
                if child.poll() is None:
                    os.kill(child.pid, signal.SIGCONT)
                    child.terminate()
                child.wait(timeout=4)
                child.stdout.close()
            self.addCleanup(close)
            self.assertEqual(child.stdout.readline(), b'ready\n')
            return child.pid

        def stopped(pid):
            os.kill(pid, signal.SIGSTOP)
            for _ in range(100):
                if Path(f'/proc/{pid}/stat').read_text().split()[2] == 'T':
                    return
                time.sleep(0.01)
            self.fail('The isolated fixture did not stop')

        def pin(pid, *, update=True):
            self.bind(str(pid), Path(f'/proc/{pid}/stat').read_text().split()[21], pin=update)

        manager_exe = self.bin / 'awesome'
        shutil.copy2(self.app, manager_exe)
        manager = launch([manager_exe])
        stopped(manager)
        self.put(self.root / 'manager', str(manager))
        resident = launch(self.arguments())
        pin(resident)
        self.accepted('pre', 16, 1, 'D:1', real_proc=True)
        self.asleep()
        stopped(resident)
        self.accepted('sleep-pre', 16, 1, 'none', real_proc=True)
        self.accepted('sleep-post', 16, 1, 'none', command='wake_observe_any', real_proc=True)
        self.sample(17, 1)
        self.accepted('sleep-post', 17, 1, 'none', real_proc=True)
        self.awake()
        self.sample(17, 3)
        os.kill(resident, signal.SIGCONT)
        self.accepted('post', 17, 3, 'none', real_proc=True)
        new_reader = launch(self.arguments())
        pin(new_reader, update=False)
        self.refused('post', real_proc=True)
