import os
import time
from unittest.mock import patch

from pose_probe import TRACE_COUNT, parse_observation, render_device
from test_pose_probe import LABEL, PoseFixture


class DisplayTraceTests(PoseFixture):
    def setUp(self):
        super().setUp()
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.command = self.bin / 'lipc-get-prop'
        self.command.write_text(f'''#!/bin/sh
[ "$#:$1:$2" = '2:com.lab126.winmgr:accelerometer' ] || exit 65
read -r counter <'{self.root}/counter' || exit 66
case "$counter" in
    1) printf 'R\\n';; 3) printf 'L';; 5) printf 'L#junk';; 7) printf 'U\\n\\n';;
    9) printf 'R'; exit 1;; 11) printf 'R\\000';; 13) printf 'U';; *) printf 'D\\n';;
esac
''')
        self.command.chmod(0o700)
        self.counter = self.root / 'counter'
        self.payload = f'''#!/bin/sh
[ "$#:$1" = '1:--display-sample' ] || exit 64
counter=0; if [ -f '{self.counter}' ]; then read -r counter <'{self.counter}'; fi
counter=$((counter + 1)); echo "$counter" >'{self.counter}'
case "$((counter % 3))" in 0) raw=15; fb=3;; 1) raw=17; fb=0;; *) raw=16; fb=1;; esac
printf 'sample %s %s %s %s\\n' "$((counter * 100))" "$((counter * 100 + 1))" "$raw" "$fb"
'''.encode()
        self.path = patch.dict(os.environ, PATH=str(self.bin) + ':' + os.environ['PATH'])
        self.path.start()
        self.addCleanup(self.path.stop)

    def trace(self, extra='', **kwargs):
        return self.execute('probe', self.payload, display_trace=True,
                            extra='trace_pause() { :; }\n' + extra, **kwargs)

    def test_independent_readings_preserve_changes_unknowns_and_exact_clock_bounds(self):
        output = self.trace()
        row = parse_observation(output, LABEL, display_trace=True)
        self.assertLess(len(output), 4096)
        self.assertEqual(row['trace_end'], 'complete')
        self.assertEqual(row['classification'], 'display_diagnostic_only')
        self.assertFalse(row['app_admitted'])
        self.assertNotIn('native_before', row)
        self.assertEqual(row['samples'][0], {'sequence': 1,
            'before': {'interval_ns': [100, 101], 'raw': 17, 'framebuffer_rotation': 0},
            'winmgr': {'interval_ns': [101, 200], 'raw': 'R'},
            'after': {'interval_ns': [200, 201], 'raw': 16, 'framebuffer_rotation': 1}})
        self.assertEqual([group['winmgr']['raw'] for group in row['samples'][:8]],
                         ['R', 'L', 'unknown', 'unknown', 'unknown', 'unknown', 'U', 'D'])
        self.assertEqual(len(row['samples']), TRACE_COUNT)
        self.assertFalse(self.stage.exists())
        self.assertEqual(self.execute('close', display_trace=True), b'closed\n')
        self.assertEqual(self.execute('absent', display_trace=True), b'absent\n')
        with self.assertRaises(ValueError):
            parse_observation(output, LABEL)

    def test_unknown_timeout_does_not_leave_the_command_or_prevent_later_samples(self):
        pid = self.root / 'held-pid'
        self.command.write_text(f'''#!/bin/sh
if [ ! -f '{pid}' ]; then echo "$$" >'{pid}'; trap '' TERM; exec sleep 20; fi
printf 'R\\n'
''')
        start = time.monotonic()
        row = parse_observation(self.trace(), LABEL, display_trace=True)
        self.assertLess(time.monotonic() - start, 8)
        self.assertEqual([entry['winmgr']['raw'] for entry in row['samples'][:2]], ['unknown', 'R'])
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid.read_text()), 0)
        self.assertFalse(self.stage.exists())

    def test_refuse_unusable_deadline_and_never_start_or_overreport_a_partial_window(self):
        self.trace('timeout() { return 64; }', expected=1, reason='timeout_unavailable')
        self.assertFalse(self.stage.exists() or self.counter.exists())
        clock = f'trace_clock() {{ if [ -f {self.counter} ]; then echo 119; else echo 100; fi; }}'
        row = parse_observation(self.trace(clock), LABEL, display_trace=True)
        self.assertEqual((row['trace_end'], len(row['samples'])), ('budget', 1))
        self.assertFalse(self.stage.exists())

    def test_malformed_or_missing_sample_and_protected_drift_never_report_observation(self):
        for response in ['exit 2', "printf 'sample 0 1 15 3\\n'", "printf 'sample 1 2 15 3\\ntrailing\\n'"]:
            self.payload = ('#!/bin/sh\n' + response + '\n').encode()
            self.assertEqual(self.trace(expected=1, reason='display_sampling'), b'')
            self.assertFalse(self.stage.exists())
        self.payload = b"#!/bin/sh\nprintf 'sample 1 2 16 1\\n'\n"
        self.assertEqual(self.trace('view() { if [ -e "$stage" ]; then echo home; else echo library; fi; }',
                                    expected=1, reason='protected_drift'), b'')
        self.assertFalse(self.stage.exists())

    def test_parser_rejects_invented_rows_boundaries_and_completed_budget(self):
        output = ('pose_display_trace ' + LABEL + '\nread 1 100 101 18 2 unknown 200 201 15 3\n'
                  'trace_end budget\nprotected_state unchanged\n')
        for old, new in [('read 1', 'read 0'), ('100 101', '102 101'), (' 200 201 ', ' 101 201 '),
                         ('18 2', '18 4'), ('15 3', '14 3'), ('unknown', 'L-and-R'), ('budget', 'complete'),
                         ('protected_state', 'unverified_state'), ('pose_display_trace', 'pose_probe')]:
            with self.assertRaises(ValueError):
                parse_observation(output.replace(old, new), LABEL, display_trace=True)
        with self.assertRaises(ValueError):
            parse_observation(output + 'x' * 4096, LABEL, display_trace=True)

    def test_trace_has_no_app_route_or_input_cmd_and_ordinary_render_stays_separate(self):
        regular, trace = (render_device(26032, display_trace=enabled) for enabled in (False, True))
        self.assertNotIn('--display-sample', regular)
        self.assertNotIn('pose_display_trace', regular)
        self.assertIn('--display-sample', trace)
        self.assertNotIn('before=$(native_read)', trace)
        self.assertNotIn('native_before %s', trace)
        self.assertNotIn('lipc-set-prop', trace)
        self.assertTrue(all('@' not in line or '$@' in line for line in trace.splitlines()))
