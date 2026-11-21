import json
import subprocess
import sys
import unittest

from run_daily_render import CLOCK, STATES, Observer, Refused, drive, install_transport, json_line, observation, public_reason, release_bounds


def encoded(record):
    return (json.dumps(record) + '\n').encode()


START = dict(ok=True, mode='XTest-button-1', clock=CLOCK, target='open-card', hold_requested_ms=50,
             down_confirmed=True, up_sent=True, buttons_clear_after=True, down_before_ns=1, down_after_ns=2,
             up_before_ns=50_000_003, up_after_ns=50_000_005)
SCENE = dict(clock=CLOCK, scene='grid', evidence='submit_upper_bound', last_negative_ns=70_000_000,
             first_complete_scene_ns=75_000_000, confirmed_scene_ns=85_000_000, submit_upper_bound_ns=80_000_000)


class SceneTests(unittest.TestCase):
    def test_submit_witness_is_only_an_upper_bound(self):
        bounds = release_bounds(encoded(START))
        result = observation(encoded(SCENE), bounds)
        self.assertEqual(result['submit_observation_upper_ns'], SCENE['submit_upper_bound_ns'] - START['up_before_ns'])
        self.assertNotIn('submit_ns', result)
        result = observation(encoded(dict(SCENE, evidence='memory_only', submit_upper_bound_ns=0)), bounds)
        self.assertIsNone(result['submit_observation_upper_ns'])

    def test_start_rejects_bad_input_or_clock(self):
        for changes in [dict(ok=1), dict(up_before_ns=True), dict(clock='CLOCK_BOOTTIME'), dict(target='nav-library'),
                        dict(down_confirmed=False), dict(up_after_ns=10), dict(down_after_ns=50_000_000),
                        dict(private_title='unknown')]:
            with self.subTest(changes=changes), self.assertRaises(Refused):
                release_bounds(encoded(dict(START, **changes)))

    def test_unrelated_or_incomplete_endpoints_refused(self):
        bounds = release_bounds(encoded(START))
        for changes in [dict(clock='CLOCK_REALTIME'), dict(scene='login'), dict(evidence='wait_completed'),
                        dict(submit_upper_bound_ns=50_000_005), dict(submit_upper_bound_ns=74_999_999),
                        dict(confirmed_scene_ns=75_000_000),
                        dict(last_negative_ns=75_001_000), dict(first_complete_scene_ns=49_000_000),
                        dict(evidence='memory_only'), dict(extra='private'), dict(confirmed_scene_ns=True)]:
            with self.subTest(changes=changes), self.assertRaises(Refused):
                observation(encoded(dict(SCENE, **changes)), bounds)

    def test_never_echo_unknown_stderr(self):
        self.assertEqual(public_reason(b'private text\nrefused native_state\n'), 'native_state')
        self.assertEqual(public_reason(b'private text\n'), 'unconfirmed')
        with self.assertRaises(Refused):
            json_line(b'{"state":"no", "state":"admitted"}')

    def test_device_script_and_binary_payload_have_separate_boundaries(self):
        label = '0' * 16
        command, header = install_transport(b'set -eu\n[ "$1:$2" = "install:' + label.encode() + b'" ]; cat\n', label)
        archive = b'\x1f\x8b\n\x00\xff'
        self.assertEqual(subprocess.check_output(['sh', '-c', command], input=header + archive), archive)


class TransactionTests(unittest.TestCase):
    def execute(self, failed='', recovering=False, absent=False, progress='no_input'):
        calls, report = [], {}

        def step(mode):
            calls.append(mode)
            if mode == failed:
                raise Refused(mode)
            if mode == 'tap':
                return encoded(START)
            if mode in STATES:
                state = 'already_absent' if absent and mode in ('stop', 'restore', 'close') else STATES[mode]
                return encoded(dict(state=state))

        class FakeObserver:
            alive = True

            def armed(self):
                step('observe_armed')

            def finish(self):
                self.alive = False
                step('observe_end')
                return encoded(SCENE)

            def running(self):
                return self.alive and failed != 'observer_exited'

            def close(self):
                step('observer_joined')

        def begin():
            step('observe')
            return FakeObserver()

        try:
            drive(step, begin, lambda: step('release') or 'released-time', report, lambda: None, recovering, progress)
        except Refused:
            pass
        return calls, report

    def test_success_and_cleanup_only(self):
        calls, report = self.execute()
        self.assertEqual(calls, 'admit stage install prepare arm observe observe_armed tap claim observe_end witness stop restore close absent release'.split())
        self.assertTrue(report['released'])
        self.assertFalse(report['recovery_required'])
        self.assertEqual(report['primary'], 'not_timed')
        self.assertIn('observation', report)
        calls, report = self.execute(recovering=True)
        self.assertEqual(calls, ['cancel', 'stop', 'restore', 'close', 'absent', 'release'])
        self.assertNotIn('observation', report)

    def test_transport_refusals_do_not_leak_a_stage_or_repeat_tap(self):
        for mode in 'admit stage install prepare arm observe observe_armed tap claim observe_end witness'.split():
            with self.subTest(mode=mode):
                calls, report = self.execute(mode)
                self.assertEqual(report['refused'], mode)
                self.assertTrue(report['released'])
                self.assertNotIn('observation', report)
                if mode == 'admit':
                    self.assertEqual(calls, ['admit', 'release'])
                else:
                    self.assertEqual(calls[-5:], ['stop', 'restore', 'close', 'absent', 'release'])
                self.assertLessEqual(calls.count('tap'), 1)
                if mode in ('observe_armed', 'tap', 'claim'):
                    self.assertLess(calls.index('observer_joined'), calls.index('stop'))

    def test_unconfirmed_close_does_not_release(self):
        for mode in ('stop', 'restore', 'close', 'absent', 'release'):
            with self.subTest(mode=mode):
                calls, report = self.execute(mode)
                self.assertTrue(report['recovery_required'])
                self.assertFalse(report['released'])
                self.assertEqual(calls[-1], mode)

    def test_dead_armed_observer_never_dispatches_the_tile(self):
        calls, report = self.execute('observer_exited')
        self.assertNotIn('tap', calls)
        self.assertNotIn('claim', calls)
        self.assertEqual(report['refused'], 'observer_exited_before_tap')
        self.assertTrue(report['released'])

    def test_lost_stage_after_navigation_is_not_native_restoration(self):
        calls, report = self.execute(recovering=True, absent=True, progress='navigation_possible')
        self.assertNotIn('release', calls)
        self.assertTrue(report['recovery_required'])
        for progress in ('no_input', 'restoration_proved'):
            with self.subTest(progress=progress):
                self.assertTrue(self.execute(recovering=True, absent=True, progress=progress)[1]['released'])

    def test_prearmed_observer_does_not_drop_buffered_result(self):
        completed = []
        for output in (b'ARMED\n' + encoded(SCENE), b'READY\n'):
            with subprocess.Popen([sys.executable, '-c', f'import sys; sys.stdout.buffer.write({output!r})'],
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, bufsize=0) as process:
                observer = Observer(process, lambda *args: completed.append(args))
                if output.startswith(b'ARMED\n'):
                    observer.armed()
                    self.assertEqual(observer.finish(), encoded(SCENE))
                else:
                    with self.assertRaises(Refused):
                        observer.armed()
        self.assertEqual(completed, [('observe_armed', 0), ('observe', 0)])


if __name__ == '__main__':
    unittest.main()
